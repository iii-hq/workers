package client

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/join"
	"github.com/iii-hq/workers/mesh/internal/meshnet"
	"github.com/iii-hq/workers/mesh/internal/node"
)

type Options struct {
	Local     *iii.Client // connected local client named "mesh"
	Namespace string      // local project namespace
	Version   string
	Dir       string // state directory; empty means $III_COMPOSE_DIR/data/mesh-client
	ConfigID  string // the mesh configuration entry, observed for changes
	// Fetch re-reads the mesh configuration entry: its mode and client section.
	Fetch func(context.Context) (mode string, c *Config, err error)
}

// Status is the client-mode answer of the local mesh::status.
type Status struct {
	Mode            string         `json:"mode"`
	HubURL          string         `json:"hub_url"`
	Node            string         `json:"node"`
	Namespace       string         `json:"namespace"`
	State           string         `json:"state"`
	Error           string         `json:"error"`
	Retrying        bool           `json:"retrying,omitempty"`       // a retry is scheduled: nothing to do but wait
	RequestedName   string         `json:"requested_name,omitempty"` // the client.name the hub last answered for
	RenameError     string         `json:"rename_error,omitempty"`   // why the machine does not have client.name
	JoinUnused      bool           `json:"join_unused,omitempty"`    // client.hub_url or client.join is not the enrollment in use: used only after a leave
	HubWantsVersion string         `json:"hub_wants_version,omitempty"`
	Shared          []SharedStatus `json:"shared"`
	Remote          []RemoteStatus `json:"remote"` // what this machine can call in the mesh
}

// LeaveResult is the answer of mesh::leave.
type LeaveResult struct {
	HubNotified bool   `json:"hub_notified"` // the hub deleted this machine's record
	Warning     string `json:"warning"`      // set when the hub may still list the machine
}

type joinState struct {
	PublicURL     string        `json:"public_url"`
	Secret        string        `json:"secret"`
	Identity      join.Identity `json:"hub_identity"`
	RequestedName string        `json:"requested_name"`
	Pending       bool          `json:"pending,omitempty"`     // saved before the hub accepted it
	JoinDigest    string        `json:"join_digest,omitempty"` // names the join value it enrolled with
}

type leaveReply struct {
	res LeaveResult
	err error
}

// leftNote is the left marker: the join value left behind, by digest ("" when
// the enrollment did not name it), and the warning shown until another joins.
type leftNote struct {
	JoinDigest string `json:"join_digest"`
	Warning    string `json:"warning,omitempty"`
}

type runner struct {
	o      Options
	dir    string
	bridge *Bridge
	// enroll reports spent when the hub may have counted a join attempt.
	enroll func(context.Context, Config, string) (st joinState, spent bool, err error)

	cfg      *Config    // last configuration read
	joined   *joinState // enrollment of the running node
	tried    string     // last join value that may have spent a hub join attempt
	triedErr error      // why tried failed; nil while it succeeded or is registering
	fetchErr error      // why the last read of the settings failed
	retry    bool       // the last pass failed in a way no event announces
	cancel   context.CancelFunc
	done     chan error
	since    time.Time            // when the running node started
	leaving  chan chan leaveReply // mesh::leave requests, served by loop
	wake     chan struct{}        // capacity 1; pending says what to do
	pending  atomic.Uint32
	dirty    chan struct{} // capacity 1: the status may have changed
	force    atomic.Bool   // publish the status even when it is unchanged
	emit     func([]byte)  // delivers one status change; nil publishes nothing

	mu        sync.Mutex
	state     string // shown while no node runs
	err       string
	hubURL    string
	restart   bool
	retrying  bool
	renaming  bool
	requested string // joined.RequestedName, for status
	renameTo  string // name the hub gave, until the node's map shows it
	renameErr string
	unused    bool // see Status.JoinUnused
	mesh      *meshnet.Mesh
	n         *node.Node
}

// What a wake asks of loop.
const (
	wakeApply uint32 = 1 << iota // the settings or the join value may have changed
	wakeSync                     // local functions changed
	wakeNode                     // the node's name or gateway changed
)

// StatusChanged is the private function the worker calls with each new mesh::status answer; the Console observes it.
const StatusChanged = "mesh::on-status-changed"

// The first retry of a failed pass and the longest wait between two.
var retryMin, retryMax = 2 * time.Second, time.Minute

func (r *runner) kick(why uint32) {
	r.pending.Or(why)
	select {
	case r.wake <- struct{}{}:
	default:
	}
}

// changed says the status may differ from the one last published.
func (r *runner) changed() {
	select {
	case r.dirty <- struct{}{}:
	default:
	}
}

// Run never returns an error for a join or network failure: compose would
// restart it in a loop and spend the hub's scarce join attempts.
func Run(ctx context.Context, o Options) error {
	if o.Dir == "" {
		o.Dir = filepath.Join(os.Getenv("III_COMPOSE_DIR"), "data", "mesh-client")
	}
	r := &runner{o: o, dir: o.Dir, bridge: NewBridge(o.Local, o.Namespace), enroll: enroll, leaving: make(chan chan leaveReply), state: "starting",
		wake: make(chan struct{}, 1), dirty: make(chan struct{}, 1)}
	r.bridge.Changed = r.changed
	defer r.bridge.Close()
	if _, err := iii.RegisterFunctionTyped(o.Local, "mesh::status", func(context.Context, struct{}) (Status, error) { return r.status(), nil }); err != nil {
		return fmt.Errorf("register mesh status: %w", err)
	}
	if _, err := iii.RegisterFunctionTyped(o.Local, "mesh::leave", func(ctx context.Context, _ struct{}) (LeaveResult, error) { return r.requestLeave(ctx) }); err != nil {
		return fmt.Errorf("register mesh leave: %w", err)
	}
	if err := r.observe(ctx); err != nil {
		return err
	}
	r.loop(ctx)
	return nil
}

// StatusEvents registers StatusChanged; current runs once a new observer is active, so one added after a status read misses nothing.
func StatusEvents(ctx context.Context, c *iii.Client, current func()) (emit func(status []byte), err error) {
	c.OnObserverAddedFor(StatusChanged, func(_ context.Context, obs *iii.Observer) error {
		go func() {
			if obs.Accepted(ctx) == nil {
				current()
			}
		}()
		return nil
	})
	if _, err = c.RegisterFunction(StatusChanged, iii.Identity, iii.RegisterFunctionOptions{Private: true, Metadata: json.RawMessage(`{"internal":true,"trace_hidden":true}`),
		Description: "Called by the mesh worker with its mesh::status answer when that answer changes."}); err != nil {
		return nil, fmt.Errorf("register mesh status events: %w", err)
	}
	return func(status []byte) {
		if c.IsObserved(StatusChanged) {
			_, _ = c.Call(ctx, iii.CallRequest{FunctionID: StatusChanged, Payload: status, Action: iii.VoidAction()})
		}
	}, nil
}

// observe turns the engine's events into wakes. No payload is read: each event only says what to read again.
func (r *runner) observe(ctx context.Context) error {
	local := r.o.Local
	internal := iii.RegisterFunctionOptions{Metadata: json.RawMessage(`{"internal":true,"trace_hidden":true}`)}
	emit, err := StatusEvents(ctx, local, func() {
		r.force.Store(true)
		r.changed()
	})
	if err != nil {
		return err
	}
	r.emit = emit
	type ack struct {
		OK bool `json:"ok"`
	}
	for id, why := range map[string]uint32{"mesh::on-config-change": wakeApply, "mesh::on-functions-change": wakeSync} {
		if _, err := iii.RegisterFunctionTyped(local, id, func(context.Context, json.RawMessage) (ack, error) {
			r.kick(why)
			return ack{true}, nil
		}, internal); err != nil {
			return fmt.Errorf("register %s: %w", id, err)
		}
	}
	for _, o := range []struct {
		source iii.Source
		target string
	}{
		{iii.Source{FunctionID: "configuration::on-updated", Namespace: "default", Deliver: iii.DeliverOutput, Filters: iii.Filters{
			"input.id":         iii.Op{"$eq": r.o.ConfigID},
			"input.event_type": iii.Op{"$in": []string{"configuration:registered", "configuration:updated", "configuration:deleted"}},
		}}, "mesh::on-config-change"},
		// client.join may be a secret:// reference: any secret change reads it again.
		{iii.Source{FunctionID: "secrets::on-changed", Deliver: iii.DeliverOutput}, "mesh::on-config-change"},
		{iii.Source{FunctionID: "engine::on-functions-changed", Deliver: iii.DeliverOutput}, "mesh::on-functions-change"},
	} {
		if _, err := local.Observe(o.source, o.target); err != nil {
			return fmt.Errorf("observe %s: %w", o.source.FunctionID, err)
		}
	}
	// Observers replay no history: read everything again after a reconnect.
	first := true // the listener is first called with the current state, which loop already acts on
	local.AddConnectionStateListener(func(s iii.ConnectionState) {
		if s == iii.StateConnected && !first {
			r.kick(wakeApply)
		}
		first = false
	})
	return nil
}

// loop is the only goroutine that starts, stops, renames and forgets the node; a timer only retries what failed.
func (r *runner) loop(ctx context.Context) {
	var publishing sync.WaitGroup
	publishing.Go(func() { r.publish(ctx) })
	defer publishing.Wait()
	serve := func(reply chan leaveReply) {
		res, err := r.leave(ctx)
		reply <- leaveReply{res, err}
		r.pending.Or(wakeApply) // a new join value may already be stored
	}
	var timer *time.Timer
	var retry <-chan time.Time
	var backoff time.Duration
	// settle starts the retry a failure asks for and publishes whether one waits.
	settle := func() {
		if !r.retry {
			// A node that has not run for retryMax yet may fail again at once: its next failure keeps backing off.
			if r.done == nil || time.Since(r.since) >= retryMax {
				backoff = 0
			}
		} else if timer == nil {
			backoff = min(max(retryMin, 2*backoff), retryMax)
			timer = time.NewTimer(backoff)
			retry = timer.C
		}
		r.mu.Lock()
		r.retrying = r.retry
		r.mu.Unlock()
		r.changed()
	}
	r.pending.Or(wakeApply)
	for {
		// A leave queued behind a pass, which can take a minute, goes first: select picks among ready cases at random.
		select {
		case reply := <-r.leaving:
			serve(reply)
		default:
		}
		if why := r.pending.Swap(0); why != 0 {
			if why&wakeApply != 0 {
				r.apply(ctx)
				// Only a whole pass ends a pending retry: another event must not push it back.
				if timer != nil {
					timer.Stop()
					timer, retry = nil, nil
				}
			} else {
				if why&wakeSync != 0 {
					r.sync(ctx)
				}
				if why&wakeNode != 0 {
					r.rename(ctx)
				}
			}
			settle()
		}
		select {
		case <-ctx.Done():
			if timer != nil {
				timer.Stop()
			}
			if r.done != nil {
				r.cancel()
				r.stop(<-r.done)
			}
			return
		case err := <-r.done:
			ran := time.Since(r.since)
			r.stop(err)
			if err == nil || errors.Is(err, node.ErrRemoved) {
				r.pending.Or(wakeApply)
				break
			}
			// Started again only after the backoff: a cause that holds would rebuild the mesh in a loop.
			if ran >= retryMax {
				backoff = 0
			}
			r.retry = true
			settle()
		case reply := <-r.leaving:
			serve(reply)
		case <-r.wake:
		case <-retry:
			r.pending.Or(wakeApply)
		}
	}
}

// publish delivers the status each time it changes. It is not loop, which a join holds for up to a minute.
func (r *runner) publish(ctx context.Context) {
	var last []byte
	for {
		select {
		case <-ctx.Done():
			return
		case <-r.dirty:
		}
		if r.emit == nil {
			continue
		}
		status, _ := json.Marshal(r.status())
		if !r.force.Swap(false) && bytes.Equal(status, last) {
			continue
		}
		last = status
		r.emit(status)
	}
}

func (r *runner) requestLeave(ctx context.Context) (LeaveResult, error) {
	reply := make(chan leaveReply, 1)
	select {
	case r.leaving <- reply:
	case <-ctx.Done():
		return LeaveResult{}, ctx.Err()
	}
	out := <-reply
	return out.res, out.err
}

// apply reads the settings and brings the node in line with them.
func (r *runner) apply(ctx context.Context) {
	r.retry = false
	mode, c, err := r.o.Fetch(ctx)
	if r.fetchErr = err; err != nil {
		r.fetchErr = fmt.Errorf("read configuration: %w", err)
	} else {
		if c == nil {
			c = &Config{}
		}
		r.mu.Lock()
		if r.cfg != nil && r.cfg.Name != c.Name {
			r.renameErr = "" // it was about the name asked before
		}
		r.cfg = c
		r.restart = mode != "client" // the worker picks hub or client when it starts
		if r.done == nil {
			r.hubURL, r.unused = c.HubURL, false // start sets both from the enrollment it finds
		}
		r.mu.Unlock()
	}
	if r.cfg == nil {
		r.fail("error", r.fetchErr)
		return
	}
	r.sync(ctx)
	if r.done == nil {
		r.start(ctx, *r.cfg)
	} else {
		r.rename(ctx)
		unused := r.isUnused(ctx, *r.cfg, r.joined)
		r.mu.Lock()
		r.unused = unused
		r.mu.Unlock()
	}
}

// isUnused says whether client.hub_url or client.join names another hub or join value than the enrollment st.
func (r *runner) isUnused(ctx context.Context, c Config, st *joinState) bool {
	if c.HubURL != "" && c.HubURL != st.PublicURL {
		return true
	}
	if c.Join == "" || st.JoinDigest == "" {
		return false
	}
	value, err := resolveJoin(ctx, r.o.Local, c.Join)
	return err == nil && joinDigest(value) != st.JoinDigest
}

// sync shows the shared workers' current functions to the hub. A running node reports what failed.
func (r *runner) sync(ctx context.Context) {
	if r.cfg == nil {
		return
	}
	err := errors.Join(r.fetchErr, r.bridge.Sync(ctx, r.cfg.Share))
	if err != nil {
		r.retry = true
	}
	if r.done != nil {
		r.mu.Lock()
		r.err = errText(err)
		r.mu.Unlock()
	}
}

const renameTimeout = 15 * time.Second

// rename asks the hub for client.name. A failure is asked again after the retry backoff: the hub may be
// unreachable, fail on its side or run an earlier build that an update replaces.
func (r *runner) rename(ctx context.Context) {
	if r.done == nil || r.cfg == nil {
		return
	}
	want := r.cfg.Name
	if want == "" || want == r.joined.RequestedName {
		return
	}
	r.mu.Lock()
	m := r.mesh
	r.mu.Unlock()
	if _, named := m.SelfName(); !named {
		return // the node's first map wakes this again
	}
	r.mu.Lock()
	r.renaming = true // the last failure stays shown while the same name is asked again
	r.mu.Unlock()
	r.changed()
	renameCtx, cancel := context.WithTimeout(ctx, renameTimeout)
	name, err := m.Rename(renameCtx, want)
	cancel()
	var refused *meshnet.RenameRefused
	switch {
	case err == nil:
		r.joined.RequestedName = want
		err = savePrivate(r.dir, "join.json", r.joined)
	case errors.As(err, &refused), errors.Is(err, meshnet.ErrRenameUnsupported):
		r.retry = true
	default:
		r.retry = true
		err = fmt.Errorf("hub not reached: %w", err)
	}
	r.mu.Lock()
	r.renaming, r.renameTo, r.renameErr, r.requested = false, name, errText(err), r.joined.RequestedName
	r.mu.Unlock()
}

const (
	removedState  = "removed by hub"
	leftState     = "left hub"
	leftMarker    = "left" // in the state directory; holds a leftNote
	logoutTimeout = 10 * time.Second
)

var errRemovedByHub = errors.New("the hub removed this machine; leave the hub, then join with a new join code")

func (r *runner) start(ctx context.Context, c Config) {
	if r.left(ctx, c) {
		return
	}
	if _, err := os.Stat(filepath.Join(r.dir, "mesh", meshnet.Removed)); err == nil {
		r.set(removedState, errRemovedByHub)
		return
	}
	// node.New refuses a build without a version; checked before joining so it
	// never joins and leaves the hub on every pass.
	if r.o.Version == "" {
		r.set("error", errors.New("binary has no version; build it with make build"))
		return
	}
	st, fresh, err := r.join(ctx, c)
	if err != nil || st == nil {
		return
	}
	unused := r.isUnused(ctx, c, st)
	r.mu.Lock()
	r.hubURL, r.unused = st.PublicURL, unused
	r.mu.Unlock()
	if fresh {
		r.set("waiting for hub", nil)
	}
	m, err := meshnet.UpUserspace(filepath.Join(r.dir, "mesh"), st.PublicURL, st.Secret, st.RequestedName, st.Identity.Control)
	refused := false
	if err == nil && fresh {
		// join.json stays pending until the hub accepts: the backend retries a
		// refused secret forever, and each refusal spends a hub join attempt.
		if err = awaitRegistration(ctx, m); err == nil {
			st.Pending = false
			err = savePrivate(r.dir, "join.json", st)
		} else if refused = errors.Is(err, errRefused); refused {
			err = errors.Join(err, os.Remove(filepath.Join(r.dir, "join.json")))
		}
		if err != nil {
			m.Close()
		}
	}
	if err != nil {
		if refused {
			r.triedErr = err
			r.set("join failed", fmt.Errorf("%w (change client.join to retry)", err))
		} else {
			// A kept pending join.json is retried: the hub may have accepted it.
			r.fail("error", fmt.Errorf("start mesh: %w", err))
		}
		return
	}
	n, err := r.newNode(m, st)
	if err != nil {
		m.Close()
		r.fail("error", err)
		return
	}
	runCtx, cancel := context.WithCancel(ctx)
	r.cancel, r.done, r.joined, r.since = cancel, make(chan error, 1), st, time.Now()
	r.mu.Lock()
	r.mesh, r.n, r.err, r.requested = m, n, "", st.RequestedName
	r.mu.Unlock()
	r.changed()
	go func(done chan<- error) { done <- n.Run(runCtx) }(r.done)
}

func (r *runner) newNode(m *meshnet.Mesh, st *joinState) (*node.Node, error) {
	url, token, err := m.Forward()
	if err != nil {
		return nil, fmt.Errorf("start forwarder: %w", err)
	}
	return node.New(node.Options{Mesh: m, HubIdentity: st.Identity.Identity, Version: r.o.Version, Workers: r.bridge, URL: url, Token: token, Changed: func() {
		r.kick(wakeNode)
		r.changed()
	}})
}

// join loads the enrollment, or enrolls with client.join. A value that may
// have spent a hub join attempt and failed is not tried again. A nil state with
// a nil error means "waiting"; fresh marks an enrollment the hub has not accepted.
func (r *runner) join(ctx context.Context, c Config) (st *joinState, fresh bool, err error) {
	path := filepath.Join(r.dir, "join.json")
	if b, err := os.ReadFile(path); err == nil {
		st := &joinState{}
		if err = json.Unmarshal(b, st); err != nil {
			r.set("error", fmt.Errorf("read join.json (leave the hub to rejoin): %w", err))
			return nil, false, err
		}
		// A lost mesh state is a new machine key, which the hub has not accepted.
		_, err = os.Stat(filepath.Join(r.dir, "mesh", "state"))
		return st, st.Pending || errors.Is(err, os.ErrNotExist), nil
	} else if !errors.Is(err, os.ErrNotExist) {
		r.fail("error", err)
		return nil, false, err
	}
	if c.Join == "" {
		r.set("waiting for join", nil)
		return nil, false, nil
	}
	value, err := resolveJoin(ctx, r.o.Local, c.Join)
	if err != nil {
		r.retry = !strings.HasPrefix(c.Join, "env://") // this process's environment never changes
		r.set("waiting for join", fmt.Errorf("resolve %s: %w", c.Join, err))
		return nil, false, err
	}
	if value == r.tried && r.triedErr != nil {
		r.set("join failed", fmt.Errorf("%w (change client.join to retry)", r.triedErr))
		return nil, false, r.triedErr
	}
	r.set("joining", nil)
	enrolled, spent, err := r.enroll(ctx, c, value)
	if err == nil {
		// Saved before registering: the hub replaces the code when it accepts the
		// node, so an interrupted join must come back with this secret.
		enrolled.Pending, enrolled.JoinDigest = true, joinDigest(value)
		err = savePrivate(r.dir, "join.json", enrolled)
	}
	if spent || err == nil {
		r.tried, r.triedErr = value, err
	}
	if err != nil {
		if spent {
			r.set("join failed", fmt.Errorf("%w (change client.join to retry)", err))
		} else if errors.As(err, new(invalidJoin)) {
			r.set("waiting for join", err) // only a save fixes it, and a save is an event
		} else {
			r.fail("waiting for join", err) // nothing counted against the hub: retried
		}
		return nil, false, err
	}
	return &enrolled, true, nil
}

// joinDigest names a join value without keeping it: a leave forgets the credential.
func joinDigest(value string) string {
	sum := sha256.Sum256([]byte(value))
	return hex.EncodeToString(sum[:])
}

// left reports whether a leave still holds. The node joins again only when
// client.join resolves to another value than the forgotten enrollment used.
func (r *runner) left(ctx context.Context, c Config) bool {
	path := filepath.Join(r.dir, leftMarker)
	b, err := os.ReadFile(path)
	if errors.Is(err, os.ErrNotExist) {
		return false
	}
	if err != nil {
		r.fail("error", err)
		return true
	}
	var note leftNote
	_ = json.Unmarshal(b, &note) // an unreadable note names no join value, like an earlier build's enrollment
	var warning error
	if note.Warning != "" {
		warning = errors.New(note.Warning)
	}
	// A leave cut short may have kept the enrollment; deleting it again is harmless.
	if err = r.forget(); err != nil {
		r.fail("error", err)
		return true
	}
	value := ""
	if c.Join != "" {
		if value, err = resolveJoin(ctx, r.o.Local, c.Join); err != nil {
			r.retry = !strings.HasPrefix(c.Join, "env://")
			r.set(leftState, errors.Join(warning, fmt.Errorf("resolve %s: %w", c.Join, err)))
			return true
		}
	}
	now := joinDigest(value)
	if note.JoinDigest == "" {
		// An enrollment saved by an earlier build does not name its join value: the first one read counts as it.
		note.JoinDigest = now
		if err = savePrivate(r.dir, leftMarker, note); err != nil {
			r.fail("error", err)
			return true
		}
	}
	if value == "" || note.JoinDigest == now {
		r.set(leftState, warning)
		return true
	}
	if err = os.Remove(path); err != nil {
		r.fail("error", err)
		return true
	}
	return false
}

// forget deletes the enrollment, so the next join is a new machine.
func (r *runner) forget() error {
	return errors.Join(os.RemoveAll(filepath.Join(r.dir, "join.json")), os.RemoveAll(filepath.Join(r.dir, "mesh")))
}

// leave logs the node out of the hub when it can, then forgets the enrollment.
// It runs on loop, so it never overlaps a pass, start or stop.
func (r *runner) leave(ctx context.Context) (LeaveResult, error) {
	b, err := os.ReadFile(filepath.Join(r.dir, "join.json"))
	if errors.Is(err, os.ErrNotExist) {
		// A refused registration deletes join.json but keeps the mesh state the hub may have removed.
		if _, e := os.Stat(filepath.Join(r.dir, "mesh")); errors.Is(e, os.ErrNotExist) {
			return LeaveResult{}, errors.New("not joined to a hub")
		}
	} else if err != nil {
		return LeaveResult{}, err
	}
	var st joinState
	_ = json.Unmarshal(b, &st) // an unreadable enrollment is forgotten too; its join value is then unknown
	_, err = os.Stat(filepath.Join(r.dir, "mesh", meshnet.Removed))
	removed := err == nil // the hub no longer lists a machine it removed
	note := leftNote{JoinDigest: st.JoinDigest}
	if note.JoinDigest == "" && removed {
		// The removal replaced the hub's code and revoked its token, so no old value can rejoin: any value may join.
		note.JoinDigest = joinDigest("")
	}
	if !removed {
		note.Warning = "The leave did not finish, so the hub may still list this machine until it is removed there."
	}
	// Written first: a leave cut short must not rejoin with the same join value.
	if err = savePrivate(r.dir, leftMarker, note); err != nil {
		return LeaveResult{}, err
	}
	logout := errors.New("the mesh is not running")
	if r.done != nil {
		r.mu.Lock()
		m := r.mesh
		r.mu.Unlock()
		logoutCtx, cancel := context.WithTimeout(ctx, logoutTimeout)
		logout = m.Logout(logoutCtx)
		cancel()
		r.cancel()
		r.stop(<-r.done)
	}
	res := LeaveResult{HubNotified: logout == nil}
	if !res.HubNotified && !removed {
		res.Warning = fmt.Sprintf("The hub was not told (%v). It keeps listing this machine until it is removed there.", logout)
	}
	if err = r.forget(); err != nil {
		return res, fmt.Errorf("delete the enrollment: %w", err)
	}
	note.Warning = res.Warning
	if err = savePrivate(r.dir, leftMarker, note); err != nil {
		return res, err
	}
	r.mu.Lock()
	r.state, r.err, r.unused = leftState, res.Warning, false
	r.mu.Unlock()
	r.changed()
	return res, nil
}

var errRefused = errors.New("hub refused the registration")

// awaitRegistration waits for the hub to accept a new node's join secret.
func awaitRegistration(ctx context.Context, m *meshnet.Mesh) error {
	deadline := time.NewTimer(time.Minute)
	defer deadline.Stop()
	changes := m.GatewayChanges()
	for {
		if _, ok := m.SelfName(); ok {
			return nil
		}
		select {
		case msg := <-m.Errors():
			return fmt.Errorf("%w: %s", errRefused, msg)
		case <-deadline.C:
			return errors.New("hub did not accept the registration within 1m")
		case <-ctx.Done():
			return ctx.Err()
		case _, open := <-changes:
			if !open {
				changes = nil // the mesh closed: the error or the deadline says why
			}
		}
	}
}

// resolveJoin reads a secret:// ref through the secrets worker (the caller is
// "mesh", so the secret's consumers must include it) or an env:// ref.
func resolveJoin(ctx context.Context, local caller, ref string) (string, error) {
	if name, ok := strings.CutPrefix(ref, "env://"); ok {
		if v := strings.TrimSpace(os.Getenv(name)); v != "" {
			return v, nil
		}
		return "", fmt.Errorf("environment variable %s is not set", name)
	}
	payload, err := json.Marshal(map[string]string{"ref": ref})
	if err != nil {
		return "", err
	}
	raw, err := local.Call(ctx, iii.CallRequest{FunctionID: "secrets::resolve", Payload: payload, Timeout: 15 * time.Second})
	if err != nil {
		return "", err
	}
	var out struct {
		Value string `json:"value"`
	}
	if err = json.Unmarshal(raw, &out); err != nil {
		return "", err
	}
	if v := strings.TrimSpace(out.Value); v != "" {
		return v, nil
	}
	return "", errors.New("secret is empty")
}

type joinArgs struct{ PublicURL, Code, Token string }

// invalidJoin is a join value or hub URL that does not parse: no retry fixes it.
type invalidJoin struct{ error }

// parseJoin reads client.join: a token carries its hub URL, a code needs client.hub_url.
func parseJoin(hubURL, value string) (joinArgs, error) {
	if strings.HasPrefix(value, "iiimj_") {
		envelope, err := join.DecodeToken(value)
		return joinArgs{PublicURL: envelope.PublicURL, Token: value}, err
	}
	if _, err := join.ParseCode(value); err != nil {
		return joinArgs{}, err
	}
	if hubURL == "" {
		return joinArgs{}, errors.New("client.hub_url is required with a join code")
	}
	return joinArgs{PublicURL: hubURL, Code: value}, nil
}

// enroll redeems the join value and pins the hub identity. Only a code's POST
// /join counts against the hub's join limit; a token is counted when it
// registers.
func enroll(ctx context.Context, c Config, value string) (joinState, bool, error) {
	args, err := parseJoin(c.HubURL, value)
	if err != nil {
		return joinState{}, false, invalidJoin{err}
	}
	name := c.Name
	if name == "" {
		host, err := os.Hostname()
		if err != nil {
			return joinState{}, false, err
		}
		name = join.RequestedName(host)
	}
	identity, err := join.FetchIdentity(ctx, args.PublicURL)
	if err != nil {
		return joinState{}, false, err
	}
	var secret string
	var pin ed25519.PublicKey
	if args.Token != "" {
		envelope, e := join.DecodeToken(args.Token)
		if e != nil {
			return joinState{}, false, e
		}
		secret, pin = envelope.Secret, envelope.Identity
	} else if secret, pin, err = join.Redeem(ctx, args.PublicURL, args.Code); err != nil {
		return joinState{}, true, err
	}
	if err = identity.Verify(pin); err != nil {
		return joinState{}, args.Code != "", err
	}
	return joinState{PublicURL: args.PublicURL, Secret: secret, Identity: identity, RequestedName: name}, args.Code != "", nil
}

func savePrivate(dir, name string, v any) error {
	b, err := json.Marshal(v)
	if err != nil {
		return err
	}
	if err = os.MkdirAll(dir, 0700); err != nil {
		return err
	}
	if err = os.Chmod(dir, 0700); err != nil {
		return err
	}
	tmp := filepath.Join(dir, "."+name)
	if err = os.WriteFile(tmp, b, 0600); err != nil {
		return err
	}
	return os.Rename(tmp, filepath.Join(dir, name))
}

// stop releases the node after Run returned err.
func (r *runner) stop(err error) {
	r.cancel()
	r.mu.Lock()
	m := r.mesh
	r.mesh, r.n = nil, nil
	r.mu.Unlock()
	_ = r.bridge.SetRuntime(context.Background(), node.Runtime{})
	m.Close()
	r.done, r.joined = nil, nil
	r.mu.Lock()
	r.renameTo, r.renameErr, r.requested = "", "", ""
	r.mu.Unlock()
	if errors.Is(err, node.ErrRemoved) {
		r.set(removedState, errRemovedByHub)
	} else if err != nil {
		r.set("error", err)
	}
	r.changed()
}

// set also publishes: loop may sit in the next step for a minute.
func (r *runner) set(state string, err error) {
	r.mu.Lock()
	r.state, r.err = state, errText(err)
	r.mu.Unlock()
	r.changed()
}

// fail is set for a failure no event announces the end of: loop tries again later.
func (r *runner) fail(state string, err error) {
	r.retry = true
	r.set(state, err)
}

func (r *runner) status() Status {
	r.mu.Lock()
	defer r.mu.Unlock()
	s := Status{Mode: "client", HubURL: r.hubURL, Namespace: r.bridge.Namespace(), State: r.state, Error: r.err, Retrying: r.retrying, RequestedName: r.requested, RenameError: r.renameErr, JoinUnused: r.unused}
	if r.mesh != nil {
		var named bool
		s.Node, named = r.mesh.SelfName()
		s.State = "connecting"
		if named {
			s.State = "connected"
		}
		if r.renaming || r.renameTo != "" && s.Node != r.renameTo {
			s.State = "renaming"
		}
		s.HubWantsVersion = r.n.WantedVersion()
	}
	if r.restart {
		s.State = "restart required"
	}
	s.Shared = r.bridge.Shared(s.Node)
	s.Remote = r.bridge.Remote()
	return s
}

func errText(err error) string {
	if err == nil {
		return ""
	}
	return err.Error()
}
