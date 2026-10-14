package main

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"net"
	"os"
	"path/filepath"
	"regexp"
	"sync"
	"syscall"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

const developmentTunnelTTL = 5 * time.Minute
const developmentTunnelRefresh = time.Minute

type DevelopmentTunnelStatus struct {
	Enabled     bool    `json:"enabled"`
	Status      string  `json:"status"`
	TunnelID    string  `json:"tunnel_id"`
	PublicURL   *string `json:"public_url"`
	WebhookURL  *string `json:"webhook_url"`
	WebhookPath string  `json:"webhook_path"`
	Error       *string `json:"error"`
}
type tunnelIntent struct {
	Enabled        bool   `json:"enabled"`
	ConsumerID     string `json:"consumer_id"`
	LeaseID        string `json:"lease_id,omitempty"`
	ReleasePending bool   `json:"release_pending,omitempty"`
}
type tunnelLease struct {
	LeaseID    string    `json:"lease_id"`
	ConsumerID string    `json:"consumer_id"`
	TunnelID   string    `json:"tunnel_id"`
	ExpiresAt  time.Time `json:"expires_at"`
}
type tunnelSnapshot struct {
	TunnelID      string        `json:"tunnel_id"`
	Status        string        `json:"status"`
	PublicURL     *string       `json:"public_url"`
	Leases        []tunnelLease `json:"leases"`
	Prerequisites *struct {
		Cloudflared struct {
			Found bool `json:"found"`
		} `json:"cloudflared"`
	} `json:"prerequisites"`
}
type tunnelRPC func(context.Context, string, any) (json.RawMessage, error)

// One lock serializes lease acquisition, renewal and release. Notifications only
// wake the loop: event payloads can arrive out of order and are never trusted as
// the current tunnel URL.
type DevelopmentTunnel struct {
	mu         sync.Mutex
	config     func(context.Context) (Config, error)
	rpc        tunnelRPC
	now        func() time.Time
	path       string
	intent     tunnelIntent
	state      DevelopmentTunnelStatus
	storageErr error
	closed     bool
	wake       chan struct{}
	done       chan struct{}
	cancel     context.CancelFunc
}

func tunnelConsumerID(namespace, configID string) string {
	sum := sha256.Sum256([]byte(namespace + "\x00" + configID))
	return "whatsapp-" + hex.EncodeToString(sum[:])
}
func newDevelopmentTunnel(ctx context.Context, config func(context.Context) (Config, error), rpc tunnelRPC) *DevelopmentTunnel {
	t := &DevelopmentTunnel{config: config, rpc: rpc, now: time.Now, wake: make(chan struct{}, 1)}
	t.intent.ConsumerID = tunnelConsumerID(Namespace(), ConfigID())
	t.state = DevelopmentTunnelStatus{Status: "stopped", TunnelID: "webhooks", WebhookPath: cloudWebhookPath()}
	current, err := config(ctx)
	if err != nil {
		t.storageErr = errors.New("development tunnel configuration is unavailable")
		t.fail("unavailable", "Development tunnel configuration is unavailable")
		return t
	}
	t.path = filepath.Join(current.DataDir, ".development-tunnel.json")
	t.storageErr = t.load()
	if t.storageErr != nil {
		t.fail("unavailable", "Development tunnel state is invalid or inaccessible; repair .development-tunnel.json in the worker data directory")
	}
	if t.intent.Enabled && t.storageErr == nil {
		t.state.Enabled = true
		t.state.Status = "starting"
	}
	return t
}
func (t *DevelopmentTunnel) load() error {
	f, err := os.OpenFile(t.path, os.O_RDONLY|syscall.O_NOFOLLOW|syscall.O_NONBLOCK, 0)
	if errors.Is(err, os.ErrNotExist) {
		return nil
	}
	if err != nil {
		return err
	}
	defer f.Close()
	info, err := f.Stat()
	if err != nil || !info.Mode().IsRegular() || info.Size() > 4096 {
		return errors.New("invalid tunnel state file")
	}
	decoder := json.NewDecoder(io.LimitReader(f, 4097))
	decoder.DisallowUnknownFields()
	var saved tunnelIntent
	if err = decoder.Decode(&saved); err != nil {
		return err
	}
	var trailing any
	if decoder.Decode(&trailing) != io.EOF || saved.ConsumerID != t.intent.ConsumerID || len(saved.LeaseID) > 128 {
		return errors.New("invalid tunnel identity")
	}
	t.intent = saved
	return f.Chmod(0600)
}
func (t *DevelopmentTunnel) save() error {
	b, err := json.Marshal(t.intent)
	if err != nil {
		return err
	}
	f, err := os.CreateTemp(filepath.Dir(t.path), ".development-tunnel-*")
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
	if err = os.Rename(name, t.path); err != nil {
		return err
	}
	dir, err := os.Open(filepath.Dir(t.path))
	if err != nil {
		return err
	}
	defer dir.Close()
	return dir.Sync()
}
func (t *DevelopmentTunnel) persist() bool {
	if t.save() != nil {
		t.fail("failed", "Unable to save development tunnel state; check the worker data directory permissions")
		return false
	}
	return true
}
func (t *DevelopmentTunnel) fail(status, message string) {
	t.state.Enabled = t.intent.Enabled
	t.state.Status = status
	t.state.PublicURL = nil
	t.state.WebhookURL = nil
	t.state.Error = &message
}
func (t *DevelopmentTunnel) stopped() {
	t.state.Enabled = t.intent.Enabled
	t.state.Status = "stopped"
	t.state.PublicURL = nil
	t.state.WebhookURL = nil
	t.state.Error = nil
}
func (t *DevelopmentTunnel) call(ctx context.Context, id string, input, output any) error {
	callCtx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	raw, err := t.rpc(callCtx, id, input)
	if err != nil {
		return err
	}
	if output != nil {
		return json.Unmarshal(raw, output)
	}
	return nil
}
func (t *DevelopmentTunnel) snapshot(ctx context.Context) (tunnelSnapshot, error) {
	var snapshot tunnelSnapshot
	err := t.call(ctx, "quick-tunnel::status", map[string]any{"tunnel_id": "webhooks"}, &snapshot)
	if err == nil && snapshot.TunnelID != "webhooks" {
		err = errors.New("unexpected tunnel target")
	}
	return snapshot, err
}
func (t *DevelopmentTunnel) ownLease(snapshot tunnelSnapshot) *tunnelLease {
	for _, lease := range snapshot.Leases {
		if lease.ConsumerID == t.intent.ConsumerID && lease.TunnelID == "webhooks" && lease.LeaseID != "" && len(lease.LeaseID) <= 128 && lease.ExpiresAt.After(t.now()) {
			return &lease
		}
	}
	return nil
}
func (t *DevelopmentTunnel) preflight(ctx context.Context) bool {
	config, err := t.config(ctx)
	if err != nil {
		t.fail("unavailable", "Development tunnel configuration is unavailable")
		return false
	}
	if !cloudStatus(config).WebhookReady {
		t.fail("unavailable", "Enable Meta Cloud API and configure App secret and Webhook verify token before starting the development tunnel")
		return false
	}
	var listener struct {
		Applied *struct {
			Host string `json:"host"`
			Port int    `json:"port"`
		} `json:"applied"`
	}
	if t.call(ctx, "http::webhook-listener::status", map[string]any{}, &listener) != nil {
		t.fail("unavailable", "The HTTP worker is unavailable; start or update it to expose the restricted webhook listener")
		return false
	}
	if listener.Applied == nil || listener.Applied.Port < 1 || listener.Applied.Port > 65535 || (listener.Applied.Host != "localhost" && !net.ParseIP(listener.Applied.Host).IsLoopback()) {
		t.fail("unavailable", "Enable the HTTP worker's restricted webhook listener on a loopback address and configure the quick-tunnel webhooks target to forward to it")
		return false
	}
	return true
}

var quickTunnelOrigin = regexp.MustCompile(`^https://[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.trycloudflare\.com$`)

func (t *DevelopmentTunnel) apply(snapshot tunnelSnapshot, lease *tunnelLease) {
	t.state.Enabled = t.intent.Enabled
	t.state.PublicURL = nil
	t.state.WebhookURL = nil
	t.state.Error = nil
	if lease == nil || !lease.ExpiresAt.After(t.now()) {
		t.fail("starting", "The development tunnel lease is missing or expired; it will be renewed automatically")
		return
	}
	switch snapshot.Status {
	case "starting", "reconnecting", "stopped":
		t.state.Status = snapshot.Status
	case "failed":
		t.fail("failed", "The quick tunnel failed; check the quick-tunnel worker and cloudflared connection")
	case "ready":
		if snapshot.PublicURL == nil || !quickTunnelOrigin.MatchString(*snapshot.PublicURL) {
			t.fail("failed", "The quick-tunnel worker returned an invalid public HTTPS URL")
			return
		}
		origin := *snapshot.PublicURL
		webhook := origin + t.state.WebhookPath
		t.state.Status = "ready"
		t.state.PublicURL = &origin
		t.state.WebhookURL = &webhook
	default:
		t.fail("unavailable", "The quick-tunnel worker returned an unsupported status; update the worker")
	}
}
func (t *DevelopmentTunnel) reconcile(ctx context.Context, renew bool) {
	if t.closed {
		t.stopped()
		return
	}
	if t.storageErr != nil {
		return
	}
	if !t.intent.Enabled {
		t.stopped()
		if t.intent.LeaseID != "" || t.intent.ReleasePending {
			t.release(ctx)
		}
		return
	}
	if !t.preflight(ctx) {
		return
	}
	snapshot, err := t.snapshot(ctx)
	if err != nil {
		t.fail("unavailable", "The quick-tunnel worker is unavailable; start or update it and try again")
		return
	}
	if snapshot.Prerequisites != nil && !snapshot.Prerequisites.Cloudflared.Found {
		t.fail("unavailable", "Install cloudflared and configure its executable in the quick-tunnel worker before starting the tunnel")
		return
	}
	lease := t.ownLease(snapshot)
	if renew && (lease == nil || lease.ExpiresAt.Sub(t.now()) <= developmentTunnelTTL/2) {
		if !t.persist() {
			return
		}
		// Persist the requested intent before acquiring. Even if the reply is lost,
		// a restart can recover the lease by this stable consumer identity.
		expiry := t.now().Add(developmentTunnelTTL).UTC().Format(time.RFC3339Nano)
		acquireErr := t.call(ctx, "quick-tunnel::acquire", map[string]any{"consumer_id": t.intent.ConsumerID, "tunnel_id": "webhooks", "expires_at": expiry}, nil)
		snapshot, err = t.snapshot(ctx)
		if err != nil {
			t.fail("unavailable", "Unable to confirm the development tunnel lease; automatic recovery will retry")
			return
		}
		lease = t.ownLease(snapshot)
		if lease == nil {
			if acquireErr != nil {
				t.fail("failed", "Unable to acquire the development tunnel lease; check quick-tunnel availability and permissions")
			} else {
				t.fail("failed", "The quick-tunnel worker did not confirm an active lease for this worker")
			}
			return
		}
	}
	if lease != nil && t.intent.LeaseID != lease.LeaseID {
		t.intent.LeaseID = lease.LeaseID
		if !t.persist() {
			return
		}
	}
	t.apply(snapshot, lease)
}
func (t *DevelopmentTunnel) release(ctx context.Context) {
	// Recover from a lost acquire response, and verify ownership immediately before
	// releasing; never trust an arbitrary lease ID from a persisted file.
	snapshot, err := t.snapshot(ctx)
	if err != nil {
		t.fail("unavailable", "Unable to release the development tunnel lease; it will expire automatically within five minutes")
		return
	}
	lease := t.ownLease(snapshot)
	if lease != nil {
		if t.call(ctx, "quick-tunnel::release", map[string]any{"lease_id": lease.LeaseID}, nil) != nil {
			t.intent.LeaseID = lease.LeaseID
			t.persist()
			t.fail("unavailable", "Unable to release the development tunnel lease; automatic cleanup will retry")
			return
		}
	}
	t.intent.LeaseID = ""
	t.intent.ReleasePending = false
	if t.persist() {
		t.stopped()
	}
}
func (t *DevelopmentTunnel) Status(ctx context.Context) (DevelopmentTunnelStatus, error) {
	if err := lockSessionContext(ctx, &t.mu); err != nil {
		return DevelopmentTunnelStatus{}, err
	}
	defer t.mu.Unlock()
	t.reconcile(ctx, false)
	return t.state, ctx.Err()
}
func (t *DevelopmentTunnel) Start(ctx context.Context) (DevelopmentTunnelStatus, error) {
	if err := lockSessionContext(ctx, &t.mu); err != nil {
		return DevelopmentTunnelStatus{}, err
	}
	defer t.mu.Unlock()
	if t.closed {
		return t.state, errors.New("worker is shutting down")
	}
	if t.storageErr != nil {
		return t.state, nil
	}
	if !t.preflight(ctx) {
		return t.state, ctx.Err()
	}
	t.intent.Enabled = true
	if !t.persist() {
		return t.state, nil
	}
	t.reconcile(ctx, true)
	return t.state, nil
}
func (t *DevelopmentTunnel) Stop(ctx context.Context) (DevelopmentTunnelStatus, error) {
	if err := lockSessionContext(ctx, &t.mu); err != nil {
		return DevelopmentTunnelStatus{}, err
	}
	defer t.mu.Unlock()
	if t.closed {
		return t.state, errors.New("worker is shutting down")
	}
	if t.storageErr != nil {
		return t.state, nil
	}
	wasEnabled := t.intent.Enabled
	t.intent.Enabled = false
	t.intent.ReleasePending = t.intent.ReleasePending || wasEnabled || t.intent.LeaseID != ""
	t.stopped()
	if !t.persist() {
		return t.state, nil
	}
	if t.intent.ReleasePending {
		t.release(ctx)
	}
	return t.state, nil
}
func (t *DevelopmentTunnel) Notify() {
	select {
	case t.wake <- struct{}{}:
	default:
	}
}
func (t *DevelopmentTunnel) Run(parent context.Context) {
	ctx, cancel := context.WithCancel(parent)
	t.cancel = cancel
	t.done = make(chan struct{})
	go func() {
		defer close(t.done)
		ticker := time.NewTicker(developmentTunnelRefresh)
		defer ticker.Stop()
		t.Notify()
		for {
			select {
			case <-ctx.Done():
				return
			case <-ticker.C:
			case <-t.wake:
			}
			if lockSessionContext(ctx, &t.mu) != nil {
				return
			}
			t.reconcile(ctx, true)
			t.mu.Unlock()
		}
	}()
}
func (t *DevelopmentTunnel) Close(ctx context.Context) error {
	if t.cancel != nil {
		t.cancel()
	}
	if t.done != nil {
		select {
		case <-t.done:
		case <-ctx.Done():
			return ctx.Err()
		}
	}
	if err := lockSessionContext(ctx, &t.mu); err != nil {
		return err
	}
	defer t.mu.Unlock()
	if t.closed {
		return nil
	}
	t.closed = true
	if t.storageErr == nil && (t.intent.Enabled || t.intent.LeaseID != "" || t.intent.ReleasePending) {
		t.release(ctx)
	}
	var err error
	if t.state.Error != nil {
		err = errors.New(*t.state.Error)
	}
	t.stopped()
	return err
}
func registerDevelopmentTunnel(ctx context.Context, client *iii.Client, manager *Manager) error {
	rpc := func(ctx context.Context, id string, input any) (json.RawMessage, error) {
		body, err := json.Marshal(input)
		if err != nil {
			return nil, err
		}
		return client.Trigger(ctx, iii.TriggerRequest{FunctionID: id, Namespace: Namespace(), Data: body, Timeout: 10 * time.Second})
	}
	var t *DevelopmentTunnel
	if manager.interfaceOnly {
		t = &DevelopmentTunnel{config: manager.ConfigContext, wake: make(chan struct{}, 1)}
		t.intent.ConsumerID = tunnelConsumerID(Namespace(), ConfigID())
	} else {
		t = newDevelopmentTunnel(ctx, manager.ConfigContext, rpc)
	}
	manager.tunnel = t
	if err := iii.RegisterFunctionTyped(client, "whatsapp::cloud::tunnel::changed", func(context.Context, callbackRequest[callbackNotification]) (callbackAccepted, error) {
		if manager.interfaceOnly {
			return callbackAccepted{}, interfaceCallbackError()
		}
		t.Notify()
		return callbackAccepted{Accepted: true}, nil
	}, iii.RegisterFunctionOptions{Metadata: json.RawMessage(`{"internal":true,"trace_hidden":true}`)}); err != nil {
		return err
	}
	config := json.RawMessage(`{"tunnel_id":"webhooks"}`)
	if err := client.RegisterTriggerNamespaced(t.intent.ConsumerID+"-changed", "quick-tunnel::changed", "whatsapp::cloud::tunnel::changed", Namespace(), config); err != nil {
		return err
	}
	if !manager.interfaceOnly {
		t.Run(ctx)
	}
	return nil
}
func registerTunnelOperations(r *Registry) {
	for _, operation := range []string{"status", "start", "stop"} {
		r.Add("whatsapp::cloud::tunnel::"+operation, func(ctx context.Context, _ map[string]any) (any, error) {
			if r.Manager.tunnel == nil {
				message := "Development tunnel integration is unavailable"
				return DevelopmentTunnelStatus{Status: "unavailable", TunnelID: "webhooks", WebhookPath: cloudWebhookPath(), Error: &message}, nil
			}
			switch operation {
			case "start":
				return r.Manager.tunnel.Start(ctx)
			case "stop":
				return r.Manager.tunnel.Stop(ctx)
			default:
				return r.Manager.tunnel.Status(ctx)
			}
		})
	}
}
