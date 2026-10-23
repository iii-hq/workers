package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"sync"
	"syscall"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/store/sqlstore"
	"go.mau.fi/whatsmeow/types"
	waLog "go.mau.fi/whatsmeow/util/log"
)

type SessionSummary struct {
	SessionID   string `json:"session_id"`
	Status      string `json:"status"`
	CreatedAtMS int64  `json:"created_at_ms"`
}
type PairingCode struct {
	Code        string `json:"code"`
	ExpiresAtMS int64  `json:"expires_at_ms"`
}
type sessionMetadata struct {
	SessionID    string `json:"session_id"`
	CreatedAtMS  int64  `json:"created_at_ms"`
	NextSequence uint64 `json:"next_sequence"`
	Backend      string `json:"backend"`
}
type Session struct {
	operations                  sync.RWMutex
	lifecycle                   sync.Mutex
	callbacks                   sync.Mutex
	mu                          sync.Mutex
	meta                        sessionMetadata
	dir, status                 string
	client                      *whatsmeow.Client
	container                   *sqlstore.Container
	inbox                       *Inbox
	cancel                      context.CancelFunc
	connectDone                 chan struct{}
	acceptEvents                bool
	qr, pairCode                *PairingCode
	pairPhone                   string
	events                      []Event
	nextSequence, reservedUntil uint64
	eventEpoch                  uint64
}
type Manager struct {
	interfaceOnly      bool
	cloudDeliverySlots chan struct{}
	storage            storageBackend
	tunnel             *DevelopmentTunnel
	mu                 sync.RWMutex
	config             Config
	sessions           map[string]*Session
	cloud              map[string]*Inbox
	closed             bool
	shutdownDone       chan struct{}
	shutdownErr        error
	lock               *os.File
	EventQueue         chan Event
	CloudEventQueue    chan CloudEvent
	cloudEpoch         uint64
}

var sessionIDPattern = regexp.MustCompile(`^[A-Za-z0-9_-]{1,64}$`)

func validSessionID(id string) error {
	if !sessionIDPattern.MatchString(id) {
		return errors.New("session_id must contain 1..64 letters, digits, underscores or hyphens")
	}
	return nil
}
func NewManager(config Config, storage storageBackend) (*Manager, error) {
	if storage == nil {
		return nil, errors.New("database worker storage is required")
	}
	if err := config.Validate(); err != nil {
		return nil, err
	}
	if err := privateInboxDirectory(config.DataDir); err != nil {
		return nil, err
	}
	lockPath := filepath.Join(config.DataDir, ".worker.lock")
	if info, err := os.Lstat(lockPath); err == nil && !info.Mode().IsRegular() {
		return nil, errors.New("worker lock must be a regular file")
	}
	lock, err := os.OpenFile(lockPath, os.O_CREATE|os.O_RDWR|syscall.O_NOFOLLOW, 0600)
	if err != nil {
		return nil, err
	}
	if err = syscall.Flock(int(lock.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		lock.Close()
		return nil, errors.New("data_dir is already in use by another WhatsApp Go worker")
	}
	if err = lock.Chmod(0600); err != nil {
		lock.Close()
		return nil, err
	}
	m := &Manager{cloudDeliverySlots: make(chan struct{}, cloudWebhookMaxConcurrentDeliveries), storage: storage, config: config, sessions: map[string]*Session{}, cloud: map[string]*Inbox{}, lock: lock, EventQueue: make(chan Event, config.MaxEvents), CloudEventQueue: make(chan CloudEvent, config.MaxEvents)}
	entries, err := os.ReadDir(config.DataDir)
	if err != nil {
		m.Shutdown(context.Background())
		return nil, err
	}
	for _, entry := range entries {
		if strings.HasPrefix(entry.Name(), ".") {
			continue
		}
		if !entry.IsDir() || validSessionID(entry.Name()) != nil {
			m.Shutdown(context.Background())
			return nil, fmt.Errorf("unexpected entry in session data_dir: %s", entry.Name())
		}
		dir := filepath.Join(config.DataDir, entry.Name())
		path := filepath.Join(dir, "metadata.json")
		if err = regularPrivateFile(path); err != nil {
			m.Shutdown(context.Background())
			return nil, errors.New("session metadata is missing or unsafe; use a new Go data_dir")
		}
		b, readErr := os.ReadFile(path)
		var meta sessionMetadata
		if readErr != nil || json.Unmarshal(b, &meta) != nil || meta.Backend != "whatsmeow" || meta.SessionID != entry.Name() || meta.NextSequence < 1 {
			m.Shutdown(context.Background())
			return nil, errors.New("incompatible session storage; whatsmeow requires a new data_dir and pairing")
		}
		inbox, openErr := newInbox(context.Background(), m.storage, dir, config.MaxInboxMessages)
		if openErr != nil {
			m.Shutdown(context.Background())
			return nil, openErr
		}
		m.sessions[meta.SessionID] = &Session{meta: meta, dir: dir, status: "stopped", inbox: inbox, nextSequence: meta.NextSequence, reservedUntil: meta.NextSequence, events: []Event{}}
	}
	return m, nil
}
func regularPrivateFile(path string) error {
	info, err := os.Lstat(path)
	if err != nil {
		return err
	}
	if !info.Mode().IsRegular() {
		return errors.New("storage files must be regular files")
	}
	return os.Chmod(path, 0600)
}
func writeMetadata(dir string, meta sessionMetadata) error {
	b, err := json.Marshal(meta)
	if err != nil {
		return err
	}
	f, err := os.CreateTemp(dir, ".metadata-*")
	if err != nil {
		return err
	}
	name := f.Name()
	defer os.Remove(name)
	if _, err = f.Write(b); err == nil {
		err = f.Sync()
	}
	closeErr := f.Close()
	if err != nil {
		return err
	}
	if closeErr != nil {
		return closeErr
	}
	if err = os.Rename(name, filepath.Join(dir, "metadata.json")); err != nil {
		return err
	}
	d, err := os.Open(dir)
	if err != nil {
		return err
	}
	defer d.Close()
	return d.Sync()
}
func (m *Manager) Config() Config { m.mu.RLock(); defer m.mu.RUnlock(); return m.config }

type contextReadMutex struct{ *sync.RWMutex }

func (mutex contextReadMutex) TryLock() bool { return mutex.TryRLock() }

func (m *Manager) ConfigContext(ctx context.Context) (Config, error) {
	if err := lockSessionContext(ctx, contextReadMutex{&m.mu}); err != nil {
		return Config{}, err
	}
	defer m.mu.RUnlock()
	return m.config, nil
}

func (m *Manager) session(id string) (*Session, error) {
	ctx, cancel := context.WithTimeout(context.Background(), inboxBackgroundTimeout)
	defer cancel()
	return m.sessionContext(ctx, id)
}

func (m *Manager) sessionContext(ctx context.Context, id string) (*Session, error) {
	if err := validSessionID(id); err != nil {
		return nil, err
	}
	if err := lockSessionContext(ctx, contextReadMutex{&m.mu}); err != nil {
		return nil, err
	}
	defer m.mu.RUnlock()
	if m.closed {
		return nil, errors.New("worker is shutting down")
	}
	s, ok := m.sessions[id]
	if !ok {
		return nil, errors.New("session not found")
	}
	return s, nil
}

// A lookup before acquiring a lifecycle lock may refer to a deleted incarnation.
func (m *Manager) currentSession(s *Session) error {
	ctx, cancel := context.WithTimeout(context.Background(), inboxBackgroundTimeout)
	defer cancel()
	return m.currentSessionContext(ctx, s)
}

func (m *Manager) currentSessionContext(ctx context.Context, s *Session) error {
	if err := lockSessionContext(ctx, contextReadMutex{&m.mu}); err != nil {
		return err
	}
	defer m.mu.RUnlock()
	if m.closed {
		return errors.New("worker is shutting down")
	}
	if m.sessions[s.meta.SessionID] != s {
		return errors.New("session was deleted or replaced")
	}
	return nil
}

// sync mutexes have no context-aware wait. Polling TryLock bounds lifecycle
// requests without leaving a goroutine that could acquire the lock after timeout.
func lockSessionContext(ctx context.Context, mutex interface{ TryLock() bool }) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	if mutex.TryLock() {
		return nil
	}
	ticker := time.NewTicker(5 * time.Millisecond)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-ticker.C:
			if err := ctx.Err(); err != nil {
				return err
			}
			if mutex.TryLock() {
				return nil
			}
		}
	}
}

func (m *Manager) lockTransition(ctx context.Context, s *Session) (func(), error) {
	if err := lockSessionContext(ctx, &s.lifecycle); err != nil {
		return nil, err
	}
	if err := lockSessionContext(ctx, &s.operations); err != nil {
		s.lifecycle.Unlock()
		return nil, err
	}
	unlock := func() { s.operations.Unlock(); s.lifecycle.Unlock() }
	if err := m.currentSessionContext(ctx, s); err != nil {
		unlock()
		return nil, err
	}
	return unlock, nil
}
func (s *Session) summary() SessionSummary {
	s.mu.Lock()
	defer s.mu.Unlock()
	return SessionSummary{s.meta.SessionID, s.status, s.meta.CreatedAtMS}
}

// A failed request must not wait again for session state to format its result.
func (s *Session) operationResult(ctx context.Context, err error) (SessionSummary, error) {
	if lockErr := lockSessionContext(ctx, &s.mu); lockErr != nil {
		if err != nil {
			return SessionSummary{}, err
		}
		return SessionSummary{}, lockErr
	}
	defer s.mu.Unlock()
	return SessionSummary{s.meta.SessionID, s.status, s.meta.CreatedAtMS}, err
}

func (m *Manager) Create(id string) (SessionSummary, error) {
	ctx, cancel := context.WithTimeout(context.Background(), inboxBackgroundTimeout)
	defer cancel()
	return m.CreateContext(ctx, id)
}

func (m *Manager) CreateContext(ctx context.Context, id string) (SessionSummary, error) {
	if err := validSessionID(id); err != nil {
		return SessionSummary{}, err
	}
	if err := lockSessionContext(ctx, &m.mu); err != nil {
		return SessionSummary{}, err
	}
	defer m.mu.Unlock()
	if m.closed {
		return SessionSummary{}, errors.New("worker is shutting down")
	}
	if _, ok := m.sessions[id]; ok {
		return SessionSummary{}, errors.New("session already exists")
	}
	if len(m.sessions) >= m.config.MaxSessions {
		return SessionSummary{}, errors.New("max_sessions reached")
	}
	dir := filepath.Join(m.config.DataDir, id)
	if err := os.Mkdir(dir, 0700); err != nil {
		return SessionSummary{}, err
	}
	meta := sessionMetadata{id, time.Now().UnixMilli(), 1, "whatsmeow"}
	if err := writeMetadata(dir, meta); err != nil {
		os.RemoveAll(dir)
		return SessionSummary{}, err
	}
	inbox, err := newInbox(ctx, m.storage, dir, m.config.MaxInboxMessages)
	if err != nil {
		os.RemoveAll(dir)
		return SessionSummary{}, err
	}
	s := &Session{meta: meta, dir: dir, status: "stopped", inbox: inbox, nextSequence: 1, reservedUntil: 1, events: []Event{}}
	m.sessions[id] = s
	return s.summary(), nil
}
func (m *Manager) Client(id string) (*whatsmeow.Client, error) {
	ctx, cancel := context.WithTimeout(context.Background(), inboxBackgroundTimeout)
	defer cancel()
	return m.ClientContext(ctx, id)
}

func (m *Manager) ClientContext(ctx context.Context, id string) (*whatsmeow.Client, error) {
	s, err := m.sessionContext(ctx, id)
	if err != nil {
		return nil, err
	}
	if err := lockSessionContext(ctx, &s.mu); err != nil {
		return nil, err
	}
	defer s.mu.Unlock()
	if !s.acceptEvents || s.status != "connected" || s.client == nil || !s.client.IsLoggedIn() || !s.client.IsConnected() {
		return nil, errors.New("session is not connected")
	}
	return s.client, nil
}
func (m *Manager) Inbox(id string) (*Inbox, error) {
	ctx, cancel := context.WithTimeout(context.Background(), inboxBackgroundTimeout)
	defer cancel()
	return m.InboxContext(ctx, id)
}

func (m *Manager) InboxContext(ctx context.Context, id string) (*Inbox, error) {
	s, err := m.sessionContext(ctx, id)
	if err != nil {
		return nil, err
	}
	return s.inbox, nil
}
func (m *Manager) RecordSent(id string, msg MessageView) error {
	ctx, cancel := context.WithTimeout(context.Background(), inboxBackgroundTimeout)
	defer cancel()
	return m.RecordSentContext(ctx, id, msg)
}

func (m *Manager) RecordSentContext(ctx context.Context, id string, msg MessageView) error {
	inbox, err := m.InboxContext(ctx, id)
	if err != nil {
		return err
	}
	return inbox.RecordContext(ctx, []MessageView{msg})
}
func (m *Manager) CloudInbox(config Config) (*Inbox, error) {
	ctx, cancel := context.WithTimeout(context.Background(), inboxBackgroundTimeout)
	defer cancel()
	return m.CloudInboxContext(ctx, config)
}

func (m *Manager) CloudInboxContext(ctx context.Context, config Config) (*Inbox, error) {
	if err := lockSessionContext(ctx, &m.mu); err != nil {
		return nil, err
	}
	defer m.mu.Unlock()
	if m.closed {
		return nil, errors.New("worker is shutting down")
	}
	if !config.Cloud.Enabled || config.Cloud != m.config.Cloud {
		return nil, errors.New("Cloud configuration is disabled or changed")
	}
	if err := config.Cloud.Validate(); err != nil {
		return nil, err
	}
	if config.Cloud.PhoneNumberID == "" {
		return nil, errors.New("Cloud API phone number is not configured")
	}
	key := config.Cloud.PhoneNumberID
	if box := m.cloud[key]; box != nil {
		return box, nil
	}
	box, err := newInbox(ctx, m.storage, filepath.Join(m.config.DataDir, ".cloud", key), m.config.MaxInboxMessages)
	if err != nil {
		return nil, err
	}
	m.cloud[key] = box
	return box, nil
}
func (m *Manager) ApplyConfig(next Config) ([]string, error) {
	ctx, cancel := context.WithTimeout(context.Background(), inboxBackgroundTimeout)
	defer cancel()
	return m.ApplyConfigContext(ctx, next)
}

func (m *Manager) ApplyConfigContext(ctx context.Context, next Config) ([]string, error) {
	_, pending, err := m.applyConfigSnapshotContext(ctx, next)
	return pending, err
}

// Return the accepted snapshot under the publication lock so callback results
// never need to reacquire a manager lock after their request has completed.
func (m *Manager) applyConfigSnapshotContext(ctx context.Context, next Config) (Config, []string, error) {
	if err := next.Validate(); err != nil {
		return Config{}, nil, err
	}
	if err := lockSessionContext(ctx, &m.mu); err != nil {
		return Config{}, nil, err
	}
	defer m.mu.Unlock()
	if m.closed {
		return Config{}, nil, errors.New("worker is shutting down")
	}
	pending := []string{}
	if next.Database != m.config.Database {
		pending = append(pending, "database")
		next.Database = m.config.Database
	}
	if next.DataDir != m.config.DataDir {
		pending = append(pending, "data_dir")
		next.DataDir = m.config.DataDir
	}
	if next.MaxEvents != m.config.MaxEvents {
		pending = append(pending, "max_events")
		next.MaxEvents = m.config.MaxEvents
	}
	// Publish retention settings only after every inbox is validated. Purging
	// multiple independent stores cannot be atomic; subsequent successful
	// writes enforce this accepted limit in their own archive transaction.
	boxes := make([]*Inbox, 0, len(m.sessions)+len(m.cloud))
	for _, s := range m.sessions {
		boxes = append(boxes, s.inbox)
	}
	for _, box := range m.cloud {
		boxes = append(boxes, box)
	}
	locked := make([]*Inbox, 0, len(boxes))
	defer func() {
		for _, box := range locked {
			box.mu.Unlock()
		}
	}()
	seen := make(map[*Inbox]bool, len(boxes))
	for _, box := range boxes {
		if box == nil {
			return Config{}, nil, errors.New("unable to apply inbox retention: inbox is unavailable")
		}
		if seen[box] {
			continue
		}
		seen[box] = true
		if err := lockSessionContext(ctx, &box.mu); err != nil {
			return Config{}, nil, err
		}
		locked = append(locked, box)
		if err := box.active(); err != nil {
			return Config{}, nil, errors.New("unable to apply inbox retention: inbox is unavailable")
		}
	}
	if err := ctx.Err(); err != nil {
		return Config{}, nil, err
	}
	for _, box := range locked {
		box.limit = next.MaxInboxMessages
	}
	if next.Cloud != m.config.Cloud {
		m.cloudEpoch++
	}
	m.config = next
	return next, pending, nil
}
func openStoredDevice(ctx context.Context, dir string, storage storageBackend) (*sqlstore.Container, error) {
	db, err := storage.Open(ctx, dir, "device")
	if err != nil {
		return nil, err
	}
	container := sqlstore.NewWithDB(db, "sqlite3", waLog.Noop)
	if err = container.Upgrade(ctx); err != nil {
		db.Close()
		return nil, err
	}
	return container, nil
}

func (m *Manager) Start(ctx context.Context, id string) (SessionSummary, error) {
	s, err := m.sessionContext(ctx, id)
	if err != nil {
		return SessionSummary{}, err
	}
	unlock, err := m.lockTransition(ctx, s)
	if err != nil {
		return s.operationResult(ctx, err)
	}
	defer unlock()
	if err := lockSessionContext(ctx, &s.mu); err != nil {
		return s.operationResult(ctx, err)
	}
	active := s.client != nil && s.acceptEvents && s.status != "failed" && s.status != "logged_out"
	s.mu.Unlock()
	if active {
		return s.operationResult(ctx, nil)
	}
	if err = m.stopSession(ctx, s, false); err != nil {
		return s.operationResult(ctx, err)
	}
	container, err := openStoredDevice(ctx, s.dir, m.storage)
	if err != nil {
		return s.operationResult(ctx, err)
	}
	device, err := container.GetFirstDevice(ctx)
	if err != nil {
		container.Close()
		return s.operationResult(ctx, err)
	}
	capture, err := s.inbox.CaptureContext(ctx)
	if err != nil {
		container.Close()
		return s.operationResult(ctx, err)
	}
	if device.ID != nil {
		if err = refreshNativeAccountContext(ctx, s, capture, device.ID.ToNonAD().String()); err != nil {
			container.Close()
			return s.operationResult(ctx, err)
		}
	}
	client := whatsmeow.NewClient(device, waLog.Noop)
	configureNativeMediaHTTP(client, func(ctx context.Context) (int, error) {
		config, err := m.ConfigContext(ctx)
		return config.MaxMediaBytes, err
	})
	runtimeCtx, cancel := context.WithCancel(context.Background())
	client.BackgroundEventCtx = runtimeCtx
	client.InitialAutoReconnect = false
	client.AddEventHandlerWithSuccessStatus(func(event any) bool { return m.handleNativeEvent(s, client, capture, event) })
	var qr <-chan whatsmeow.QRChannelItem
	if device.ID == nil {
		qr, err = client.GetQRChannel(runtimeCtx)
		if err != nil {
			cancel()
			container.Close()
			return s.operationResult(ctx, err)
		}
	}
	if err := lockSessionContext(ctx, &s.mu); err != nil {
		cancel()
		container.Close()
		return s.operationResult(ctx, err)
	}
	s.client = client
	s.container = container
	s.cancel = cancel
	s.status = "starting"
	s.acceptEvents = true
	s.connectDone = make(chan struct{})
	done := s.connectDone
	s.mu.Unlock()
	if qr != nil {
		go m.consumeQR(runtimeCtx, s, client, qr)
	}
	go func() {
		defer close(done)
		timer := time.AfterFunc(time.Duration(m.Config().OperationTimeoutSecs)*time.Second, cancel)
		err := client.ConnectContext(runtimeCtx)
		timer.Stop()
		if err != nil {
			cancel()
			m.connectionFailed(s, client)
		}
	}()
	return s.operationResult(ctx, nil)
}

func refreshNativeAccountContext(ctx context.Context, s *Session, capture *InboxCapture, account string) error {
	if err := s.inbox.SetAccountContext(ctx, account); err != nil {
		return err
	}
	next, err := s.inbox.CaptureContext(ctx)
	if err != nil {
		return err
	}
	if next.generation != capture.generation {
		if err := lockSessionContext(ctx, &s.mu); err != nil {
			return err
		}
		s.events = []Event{}
		s.eventEpoch++
		s.mu.Unlock()
	}
	*capture = *next
	return nil
}

func (m *Manager) connectionFailed(s *Session, client *whatsmeow.Client) {
	s.callbacks.Lock()
	defer s.callbacks.Unlock()
	s.mu.Lock()
	if s.client != client || !s.acceptEvents {
		s.mu.Unlock()
		return
	}
	s.status = "failed"
	s.qr, s.pairCode = nil, nil
	s.pairPhone = ""
	s.acceptEvents = false
	s.mu.Unlock()
	_ = m.emit(s, "session.failed", map[string]any{"reason": "connection failed"})
}
func (m *Manager) consumeQR(ctx context.Context, s *Session, client *whatsmeow.Client, qr <-chan whatsmeow.QRChannelItem) {
	for {
		select {
		case <-ctx.Done():
			return
		case item, ok := <-qr:
			if !ok {
				return
			}
			s.callbacks.Lock()
			s.mu.Lock()
			if s.client != client || !s.acceptEvents {
				s.mu.Unlock()
				s.callbacks.Unlock()
				return
			}
			if item.Event == whatsmeow.QRChannelEventCode {
				s.qr = &PairingCode{item.Code, time.Now().Add(item.Timeout).UnixMilli()}
				s.status = "pairing"
			} else {
				s.qr = nil
			}
			s.mu.Unlock()
			if item.Event == whatsmeow.QRChannelEventCode {
				_ = m.emit(s, "pairing.qr", map[string]any{"available": true})
			}
			s.callbacks.Unlock()
		}
	}
}

// Caller holds the lifecycle and operations locks. No callback waits for either.
func (m *Manager) stopSession(ctx context.Context, s *Session, logout bool) error {
	if err := lockSessionContext(ctx, &s.callbacks); err != nil {
		return err
	}
	if err := lockSessionContext(ctx, &s.mu); err != nil {
		s.callbacks.Unlock()
		return err
	}
	client, cancel, done, container := s.client, s.cancel, s.connectDone, s.container
	// Invalidation can wait on an archive operation. Complete that fallible
	// step before disabling callbacks on a still-connected account.
	if err := s.inbox.InvalidateContext(ctx); err != nil {
		s.mu.Unlock()
		s.callbacks.Unlock()
		return err
	}
	s.status = "stopping"
	s.acceptEvents = false
	s.qr, s.pairCode = nil, nil
	s.pairPhone = ""
	s.mu.Unlock()
	s.callbacks.Unlock()
	var logoutErr error
	if logout && client != nil && client.IsLoggedIn() {
		logoutErr = client.Logout(ctx)
	}
	if cancel != nil {
		cancel()
	}
	if done != nil {
		select {
		case <-done:
		case <-ctx.Done():
			return ctx.Err()
		}
	}
	if client != nil {
		client.Disconnect()
		client.RemoveEventHandlers()
	}
	if container != nil {
		if err := container.Close(); err != nil {
			return err
		}
	}
	if err := lockSessionContext(ctx, &s.mu); err != nil {
		return err
	}
	s.client, s.container, s.cancel, s.connectDone = nil, nil, nil, nil
	s.mu.Unlock()
	if logout {
		if err := m.storage.ClearDevice(ctx, s.dir); err != nil {
			return err
		}

		if err := s.inbox.ClearContext(ctx); err != nil {
			return err
		}
		if err := lockSessionContext(ctx, &s.mu); err != nil {
			return err
		}
		s.events = []Event{}
		s.eventEpoch++
		s.mu.Unlock()
	}
	if err := lockSessionContext(ctx, &s.mu); err != nil {
		return err
	}
	s.status = "stopped"
	if logout {
		s.status = "logged_out"
	}
	s.mu.Unlock()
	if logoutErr != nil {
		return errors.New("local credentials cleared, but remote unlink could not be confirmed; remove the linked device on your phone")
	}
	return nil
}
func (m *Manager) Stop(ctx context.Context, id string, logout bool) (SessionSummary, error) {
	config, err := m.ConfigContext(ctx)
	if err != nil {
		return SessionSummary{}, err
	}
	ctx, cancel := context.WithTimeout(ctx, time.Duration(config.ShutdownTimeoutSecs)*time.Second)
	defer cancel()
	s, err := m.sessionContext(ctx, id)
	if err != nil {
		return SessionSummary{}, err
	}
	unlock, err := m.lockTransition(ctx, s)
	if err != nil {
		return s.operationResult(ctx, err)
	}
	defer unlock()
	err = m.stopSession(ctx, s, logout)
	return s.operationResult(ctx, err)
}
func (m *Manager) Delete(ctx context.Context, id string) (SessionSummary, error) {
	config, err := m.ConfigContext(ctx)
	if err != nil {
		return SessionSummary{}, err
	}
	ctx, cancel := context.WithTimeout(ctx, time.Duration(config.ShutdownTimeoutSecs)*time.Second)
	defer cancel()
	s, err := m.sessionContext(ctx, id)
	if err != nil {
		return SessionSummary{}, err
	}
	unlock, err := m.lockTransition(ctx, s)
	if err != nil {
		return s.operationResult(ctx, err)
	}
	defer unlock()
	if err = m.stopSession(ctx, s, false); err != nil {
		return s.operationResult(ctx, err)
	}
	if err := lockSessionContext(ctx, &m.mu); err != nil {
		return s.operationResult(ctx, err)
	}
	defer m.mu.Unlock()
	if err = m.storage.ClearDevice(ctx, s.dir); err != nil {
		return s.operationResult(ctx, err)
	}
	if err = s.inbox.ClearContext(ctx); err != nil {
		return s.operationResult(ctx, err)
	}
	if err = s.inbox.Close(); err != nil {
		return s.operationResult(ctx, err)
	}
	if err = os.RemoveAll(s.dir); err != nil {
		return s.operationResult(ctx, err)
	}
	delete(m.sessions, id)
	return s.operationResult(ctx, nil)
}
func (m *Manager) Shutdown(ctx context.Context) error {
	m.mu.Lock()
	if m.shutdownDone == nil {
		m.closed = true
		m.shutdownDone = make(chan struct{})
		go m.finishShutdown()
	}
	done := m.shutdownDone
	m.mu.Unlock()
	select {
	case <-done:
		m.mu.RLock()
		defer m.mu.RUnlock()
		return m.shutdownErr
	case <-ctx.Done():
		return ctx.Err()
	}
}

func (m *Manager) finishShutdown() {
	m.mu.RLock()
	sessions := make([]*Session, 0, len(m.sessions))
	for _, s := range m.sessions {
		sessions = append(sessions, s)
	}
	cloud := m.cloud
	m.mu.RUnlock()
	var result error
	for _, s := range sessions {
		s.lifecycle.Lock()
		s.operations.Lock()
		result = errors.Join(result, m.stopSession(context.Background(), s, false), s.inbox.Close())
		s.operations.Unlock()
		s.lifecycle.Unlock()
	}
	for _, box := range cloud {
		result = errors.Join(result, box.Close())
	}
	if m.lock != nil {
		result = errors.Join(result, m.lock.Close())
	}
	m.mu.Lock()
	m.shutdownErr = result
	close(m.shutdownDone)
	m.mu.Unlock()
}
func (m *Manager) PairCode(ctx context.Context, id, phone string) (PairingCode, error) {
	if len(phone) < 7 || len(phone) > 15 || strings.Trim(phone, "0123456789") != "" {
		return PairingCode{}, errors.New("phone_number must contain 7 to 15 digits including country code")
	}
	s, err := m.sessionContext(ctx, id)
	if err != nil {
		return PairingCode{}, err
	}
	unlock, err := m.lockTransition(ctx, s)
	if err != nil {
		return PairingCode{}, err
	}
	defer unlock()
	if err := lockSessionContext(ctx, &s.mu); err != nil {
		return PairingCode{}, err
	}
	client := s.client
	cached, previous := s.pairCode, s.pairPhone
	s.mu.Unlock()
	if cached != nil && cached.ExpiresAtMS > time.Now().UnixMilli() {
		if previous != phone {
			return PairingCode{}, errors.New("a pairing code for another number is still active")
		}
		return *cached, nil
	}
	if client == nil || !client.IsConnected() || client.IsLoggedIn() {
		return PairingCode{}, errors.New("start an unpaired session and wait for pairing before requesting a code")
	}
	code, err := client.PairPhone(ctx, phone, true, whatsmeow.PairClientChrome, "Chrome (Linux)")
	if err != nil {
		return PairingCode{}, errors.New("unable to request pairing code")
	}
	result := PairingCode{code, time.Now().Add(160 * time.Second).UnixMilli()}
	if err := lockSessionContext(ctx, &s.mu); err != nil {
		return PairingCode{}, err
	}
	if s.client != client || !s.acceptEvents || s.status == "connected" {
		s.mu.Unlock()
		return PairingCode{}, errors.New("session changed while requesting pairing code")
	}
	s.pairCode = &result
	s.pairPhone = phone
	s.mu.Unlock()
	return result, nil
}
func registerSessions(r *Registry) {
	m := r.Manager
	r.Add("whatsapp::sessions::create", func(ctx context.Context, v map[string]any) (any, error) {
		return m.CreateContext(ctx, stringArg(v, "session_id"))
	})
	r.Add("whatsapp::sessions::list", func(ctx context.Context, _ map[string]any) (any, error) {
		if err := lockSessionContext(ctx, contextReadMutex{&m.mu}); err != nil {
			return nil, err
		}
		defer m.mu.RUnlock()
		out := make([]SessionSummary, 0, len(m.sessions))
		for _, s := range m.sessions {
			summary, err := s.operationResult(ctx, nil)
			if err != nil {
				return nil, err
			}
			out = append(out, summary)
		}
		sort.Slice(out, func(i, j int) bool { return out[i].SessionID < out[j].SessionID })
		return out, nil
	})
	r.Add("whatsapp::sessions::get", func(ctx context.Context, v map[string]any) (any, error) {
		s, err := m.sessionContext(ctx, stringArg(v, "session_id"))
		if err != nil {
			return nil, err
		}
		return s.operationResult(ctx, nil)
	})
	r.Add("whatsapp::sessions::start", func(ctx context.Context, v map[string]any) (any, error) {
		return m.Start(ctx, stringArg(v, "session_id"))
	})
	r.Add("whatsapp::sessions::stop", func(ctx context.Context, v map[string]any) (any, error) {
		return m.Stop(ctx, stringArg(v, "session_id"), false)
	})
	r.Add("whatsapp::sessions::logout", func(ctx context.Context, v map[string]any) (any, error) {
		return m.Stop(ctx, stringArg(v, "session_id"), true)
	})
	r.Add("whatsapp::sessions::delete", func(ctx context.Context, v map[string]any) (any, error) {
		return m.Delete(ctx, stringArg(v, "session_id"))
	})
	r.Add("whatsapp::sessions::qr", func(ctx context.Context, v map[string]any) (any, error) {
		s, err := m.sessionContext(ctx, stringArg(v, "session_id"))
		if err != nil {
			return nil, err
		}
		if err := lockSessionContext(ctx, &s.mu); err != nil {
			return nil, err
		}
		defer s.mu.Unlock()
		if s.qr == nil || s.qr.ExpiresAtMS <= time.Now().UnixMilli() {
			return nil, nil
		}
		return *s.qr, nil
	})
	r.Add("whatsapp::sessions::pair_code", func(ctx context.Context, v map[string]any) (any, error) {
		return m.PairCode(ctx, stringArg(v, "session_id"), stringArg(v, "phone_number"))
	})
	r.Add("whatsapp::events::poll", func(ctx context.Context, v map[string]any) (any, error) {
		return m.PollContext(ctx, stringArg(v, "session_id"), uint64(intArg(v, "after_sequence", 0)), intArg(v, "limit", 100))
	})
	r.Add("whatsapp::chats::list", func(ctx context.Context, v map[string]any) (any, error) {
		box, err := m.InboxContext(ctx, stringArg(v, "session_id"))
		if err != nil {
			return nil, err
		}
		return box.ListChatsContext(ctx, stringArg(v, "query"), stringArg(v, "cursor"), intArg(v, "limit", 100))
	})
	r.Add("whatsapp::messages::list", func(ctx context.Context, v map[string]any) (any, error) {
		box, err := m.InboxContext(ctx, stringArg(v, "session_id"))
		if err != nil {
			return nil, err
		}
		return box.ListMessagesContext(ctx, stringArg(v, "chat_id"), stringArg(v, "cursor"), intArg(v, "limit", 50))
	})
	r.Add("whatsapp::messages::request_history", func(ctx context.Context, v map[string]any) (any, error) {
		id := stringArg(v, "session_id")
		client, err := m.ClientContext(ctx, id)
		if err != nil {
			return nil, err
		}
		box, err := m.InboxContext(ctx, id)
		if err != nil {
			return nil, err
		}
		chat, err := parseJID(stringArg(v, "chat_id"))
		if err != nil {
			return nil, err
		}
		count := intArg(v, "count", 50)
		if count < 1 || count > 100 {
			return nil, errors.New("count must be 1..100")
		}
		oldest, err := box.OldestContext(ctx, chat.String())
		if err != nil {
			return nil, err
		}
		if oldest.MessageID == "" {
			return nil, errors.New("history requests require an existing local message")
		}
		response, err := client.SendPeerMessage(ctx, client.BuildHistorySyncRequest(&types.MessageInfo{MessageSource: types.MessageSource{Chat: chat, IsFromMe: oldest.FromMe}, ID: oldest.MessageID, Timestamp: time.UnixMilli(oldest.TimestampMS)}, count))
		if err != nil {
			return nil, err
		}
		return map[string]any{"request_id": response.ID, "requested_count": count}, nil
	})
}
