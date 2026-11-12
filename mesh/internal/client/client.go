package client

import (
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
	HubWantsVersion string         `json:"hub_wants_version,omitempty"`
	Shared          []SharedStatus `json:"shared"`
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

	cfg      *Config // last configuration read
	started  *Config // configuration the running node was started with
	tried    string  // last join value that may have spent a hub join attempt
	triedErr error   // why tried failed; nil while it succeeded or is registering
	cancel   context.CancelFunc
	done     chan error
	leaving  chan chan leaveReply // mesh::leave requests, served by loop

	mu      sync.Mutex
	state   string // shown while no node runs
	err     string
	hubURL  string
	restart bool
	mesh    *meshnet.Mesh
	n       *node.Node
}

// Run never returns an error for a join or network failure: compose would
// restart it in a loop and spend the hub's scarce join attempts.
func Run(ctx context.Context, o Options) error {
	if o.Dir == "" {
		o.Dir = filepath.Join(os.Getenv("III_COMPOSE_DIR"), "data", "mesh-client")
	}
	r := &runner{o: o, dir: o.Dir, bridge: NewBridge(o.Local, o.Namespace), enroll: enroll, leaving: make(chan chan leaveReply), state: "starting"}
	defer r.bridge.Close()
	if _, err := iii.RegisterFunctionTyped(o.Local, "mesh::status", func(context.Context, struct{}) (Status, error) { return r.status(), nil }); err != nil {
		return fmt.Errorf("register mesh status: %w", err)
	}
	if _, err := iii.RegisterFunctionTyped(o.Local, "mesh::leave", func(ctx context.Context, _ struct{}) (LeaveResult, error) { return r.requestLeave(ctx) }); err != nil {
		return fmt.Errorf("register mesh leave: %w", err)
	}
	r.loop(ctx, 5*time.Second)
	return nil
}

// loop is the only goroutine that starts, stops and forgets the node.
func (r *runner) loop(ctx context.Context, every time.Duration) {
	ticker := time.NewTicker(every)
	defer ticker.Stop()
	serve := func(reply chan leaveReply) {
		res, err := r.leave(ctx)
		reply <- leaveReply{res, err}
	}
	for {
		// A leave queued behind a tick, which can take a minute, goes first: select picks among ready cases at random.
		select {
		case reply := <-r.leaving:
			serve(reply)
		default:
			r.tick(ctx)
		}
		select {
		case <-ctx.Done():
			if r.done != nil {
				r.cancel()
				r.stop(<-r.done)
			}
			return
		case err := <-r.done:
			r.stop(err)
		case reply := <-r.leaving:
			serve(reply)
		case <-ticker.C:
		}
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

func (r *runner) tick(ctx context.Context) {
	mode, c, fetchErr := r.o.Fetch(ctx)
	if fetchErr != nil {
		fetchErr = fmt.Errorf("read configuration: %w", fetchErr)
	} else {
		if c == nil {
			c = &Config{}
		}
		r.cfg = c
		changed := r.started != nil && (c.HubURL != r.started.HubURL || c.Join != r.started.Join || c.Name != r.started.Name)
		r.mu.Lock()
		r.restart = mode != "client" || changed
		if r.started == nil {
			r.hubURL = c.HubURL
		}
		r.mu.Unlock()
	}
	if r.cfg == nil {
		r.set("error", fetchErr)
		return
	}
	err := errors.Join(fetchErr, r.bridge.Sync(ctx, r.cfg.Share))
	if r.done != nil {
		r.mu.Lock()
		r.err = errText(err)
		r.mu.Unlock()
	} else {
		r.start(ctx, *r.cfg)
	}
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
	// never joins and leaves the hub every tick.
	if r.o.Version == "" {
		r.set("error", errors.New("binary has no version; build it with make build"))
		return
	}
	st, fresh, err := r.join(ctx, c)
	if err != nil || st == nil {
		return
	}
	r.mu.Lock()
	r.hubURL = st.PublicURL
	r.mu.Unlock()
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
			// A kept pending join.json is retried next tick: the hub may have accepted it.
			r.set("error", fmt.Errorf("start mesh: %w", err))
		}
		return
	}
	n, err := r.newNode(m, st)
	if err != nil {
		m.Close()
		r.set("error", err)
		return
	}
	runCtx, cancel := context.WithCancel(ctx)
	r.cancel, r.done, r.started = cancel, make(chan error, 1), &c
	r.mu.Lock()
	r.mesh, r.n, r.err = m, n, ""
	r.mu.Unlock()
	go func(done chan<- error) { done <- n.Run(runCtx) }(r.done)
}

func (r *runner) newNode(m *meshnet.Mesh, st *joinState) (*node.Node, error) {
	url, err := m.Forward()
	if err != nil {
		return nil, fmt.Errorf("start forwarder: %w", err)
	}
	return node.New(node.Options{Mesh: m, HubIdentity: st.Identity.Identity, Version: r.o.Version, Workers: r.bridge, URL: url})
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
		r.set("error", err)
		return nil, false, err
	}
	if c.Join == "" {
		r.set("waiting for join", nil)
		return nil, false, nil
	}
	value, err := resolveJoin(ctx, r.o.Local, c.Join)
	if err != nil {
		r.set("waiting for join", fmt.Errorf("resolve %s: %w", c.Join, err))
		return nil, false, err
	}
	if value == r.tried && r.triedErr != nil {
		r.set("join failed", fmt.Errorf("%w (change client.join to retry)", r.triedErr))
		return nil, false, r.triedErr
	}
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
		} else {
			r.set("waiting for join", err) // nothing counted against the hub: retried next tick
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
		r.set("error", err)
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
		r.set("error", err)
		return true
	}
	value := ""
	if c.Join != "" {
		if value, err = resolveJoin(ctx, r.o.Local, c.Join); err != nil {
			r.set(leftState, errors.Join(warning, fmt.Errorf("resolve %s: %w", c.Join, err)))
			return true
		}
	}
	now := joinDigest(value)
	if note.JoinDigest == "" {
		// An enrollment saved by an earlier build does not name its join value: the first one read counts as it.
		note.JoinDigest = now
		if err = savePrivate(r.dir, leftMarker, note); err != nil {
			r.set("error", err)
			return true
		}
	}
	if value == "" || note.JoinDigest == now {
		r.set(leftState, warning)
		return true
	}
	if err = os.Remove(path); err != nil {
		r.set("error", err)
		return true
	}
	return false
}

// forget deletes the enrollment, so the next join is a new machine.
func (r *runner) forget() error {
	return errors.Join(os.RemoveAll(filepath.Join(r.dir, "join.json")), os.RemoveAll(filepath.Join(r.dir, "mesh")))
}

// leave logs the node out of the hub when it can, then forgets the enrollment.
// It runs on loop, so it never overlaps tick, start or stop.
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
	r.state, r.err, r.restart = leftState, res.Warning, false // nothing runs, so no changed connection waits for a restart
	r.mu.Unlock()
	return res, nil
}

var errRefused = errors.New("hub refused the registration")

// awaitRegistration waits for the hub to accept a new node's join secret.
func awaitRegistration(ctx context.Context, m *meshnet.Mesh) error {
	deadline := time.NewTimer(time.Minute)
	defer deadline.Stop()
	poll := time.NewTicker(100 * time.Millisecond)
	defer poll.Stop()
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
		case <-poll.C:
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
		return joinState{}, false, err
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
	r.done, r.started = nil, nil
	if errors.Is(err, node.ErrRemoved) {
		r.set(removedState, errRemovedByHub)
	} else if err != nil {
		r.set("error", err)
	}
}

func (r *runner) set(state string, err error) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.state, r.err = state, errText(err)
}

func (r *runner) status() Status {
	r.mu.Lock()
	defer r.mu.Unlock()
	s := Status{Mode: "client", HubURL: r.hubURL, Namespace: r.bridge.Namespace(), State: r.state, Error: r.err}
	if r.mesh != nil {
		var named bool
		s.Node, named = r.mesh.SelfName()
		s.State = "connecting"
		if named {
			s.State = "connected"
		}
		s.HubWantsVersion = r.n.WantedVersion()
	}
	if r.restart {
		s.State = "restart required"
	}
	s.Shared = r.bridge.Shared(s.Node)
	return s
}

func errText(err error) string {
	if err == nil {
		return ""
	}
	return err.Error()
}
