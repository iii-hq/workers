package hub

import (
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"reflect"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/coordinator"
	"github.com/iii-hq/workers/mesh/internal/gateway"
	"github.com/iii-hq/workers/mesh/internal/meshnet"
	"github.com/iii-hq/workers/mesh/internal/node"
	"tailscale.com/tsnet"
)

type nodeCallRPC struct {
	mu         sync.Mutex // the reconciler calls from its own goroutine
	status     node.Status
	verifier   *node.Verifier
	calls      []string
	fail       string
	workers    node.WorkersRequest
	hubVersion string
	statuses   int
	statusErr  error  // what a status call answers instead of the status
	onStatus   func() // runs once, during the next status call, after its answer is chosen, under mu
	catalog    *node.Catalog
}

func (r *nodeCallRPC) statusCalls() int {
	r.mu.Lock()
	defer r.mu.Unlock()
	return r.statuses
}

func (r *nodeCallRPC) Call(_ context.Context, req iii.CallRequest) (json.RawMessage, error) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if req.Namespace != "default" || !strings.HasPrefix(req.FunctionID, "laptop::mesh-node::") {
		return nil, errors.New("incorrect control namespace or prefix")
	}
	fn := strings.TrimPrefix(req.FunctionID, "laptop::")
	var signed node.Signed
	var wire any
	json.Unmarshal(req.Payload, &wire)
	b, _ := json.Marshal(wire)
	json.Unmarshal(b, &signed)
	payload, err := r.verifier.Verify(fn, signed, time.Now())
	if err != nil {
		return nil, err
	}
	if fn == "mesh-node::status" {
		var request node.StatusRequest
		json.Unmarshal(payload, &request)
		r.hubVersion = request.HubVersion
		r.statuses++
		err := r.statusErr
		if f := r.onStatus; f != nil {
			r.onStatus = nil
			f()
		}
		if err != nil {
			return nil, err
		}
		return json.Marshal(r.status)
	}
	r.calls = append(r.calls, strings.TrimPrefix(fn, "mesh-node::"))
	if r.fail == fn {
		return nil, errors.New("apply failed")
	}
	if fn == "mesh-node::catalog::apply" {
		r.catalog = &node.Catalog{}
		json.Unmarshal(payload, r.catalog)
		return json.RawMessage(`{}`), nil
	}
	previous := r.workers.Namespace
	json.Unmarshal(payload, &r.workers)
	if r.workers.Namespace != previous {
		r.status.Catalog = "" // as the bridge does when its runtime moves
	}
	return json.Marshal(r.status)
}
func nodeCallHub(t *testing.T) (*Hub, *nodeCallRPC) {
	t.Helper()
	h, r, up := idleNodeCallHub(t)
	up()
	return h, r
}

// idleNodeCallHub is nodeCallHub before its peer joins; up joins it.
func idleNodeCallHub(t *testing.T) (*Hub, *nodeCallRPC, func()) {
	t.Helper()
	c, err := coordinator.New(t.TempDir(), func(string, string) (string, error) { return "", nil })
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { c.Close() })
	mux := http.NewServeMux()
	c.Handle(mux)
	front := httptest.NewServer(mux)
	t.Cleanup(front.Close)
	c.Relay(front.URL)
	shim, err := meshnet.NewShim(front.URL, c.Public())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { shim.Close() })
	peer := &tsnet.Server{Dir: t.TempDir(), Hostname: "laptop", ControlURL: shim.URL(), AuthKey: rand.Text(), Logf: func(string, ...any) {}, UserLogf: func(string, ...any) {}}
	t.Cleanup(func() { peer.Close() })
	r := &nodeCallRPC{status: node.Status{Name: "laptop", OS: "linux", Arch: "amd64", Version: "0.1.0", Workers: []node.WorkerStatus{}}, verifier: node.NewVerifier(c.Identity().Public().(ed25519.PublicKey), time.Minute)}
	h := &Hub{Coordinator: c, Namespace: "project", Version: "0.2.0", config: DefaultConfig(), nodeClient: r}
	return h, r, func() {
		t.Helper()
		ctx, cancel := context.WithTimeout(t.Context(), 10*time.Second)
		defer cancel()
		if _, err := peer.Up(ctx); err != nil {
			t.Fatal(err)
		}
	}
}
func TestVersionMismatchSharesNothing(t *testing.T) {
	h, r := nodeCallHub(t)
	// The advice names no side: the node may be the newer one, or the hub unstamped.
	if err := h.OnNodeOnline(t.Context(), "laptop"); err == nil || err.Error() != `node runs mesh "0.1.0", the hub runs "0.2.0": run one mesh version on both` {
		t.Fatalf("mismatch: %v", err)
	}
	if len(r.calls) != 0 || r.hubVersion != "0.2.0" {
		t.Fatalf("calls %v hub version %q", r.calls, r.hubVersion)
	}
	if h.Status().Nodes.Outdated != 1 {
		t.Fatal("outdated node missing")
	}
	r.status.Version = "0.2.0"
	// A worker sent without functions (null) still lists [] on the hub.
	r.status.Workers = []node.WorkerStatus{{Name: "ide", State: "shared", Functions: []string{"coder::read-file", "shell::exec"}}, {Name: "probe", State: "not running"}}
	if err := h.OnNodeOnline(t.Context(), "laptop"); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(r.calls, []string{"workers::apply"}) || r.workers.Namespace != "project" {
		t.Fatalf("calls %v apply %+v", r.calls, r.workers)
	}
	if h.Status().Nodes.Outdated != 0 {
		t.Fatal("updated node still outdated")
	}
	list, _ := h.ListNodes(t.Context(), struct{}{})
	if len(list) != 1 || list[0].Version != "0.2.0" || list[0].OS != "linux" || list[0].Arch != "amd64" {
		t.Fatalf("list %+v", list)
	}
	if workers, _ := json.Marshal(list[0].Workers); string(workers) != `[{"name":"ide","state":"shared","functions":["laptop::coder::read-file","laptop::shell::exec"]},{"name":"probe","state":"not running","functions":[]}]` {
		t.Fatalf("workers %s", workers)
	}
}
func TestOnlineNodeWorkersRefresh(t *testing.T) {
	defer func(d time.Duration) { nodeRefreshDelay = d }(nodeRefreshDelay)
	nodeRefreshDelay = time.Millisecond
	h, r := nodeCallHub(t)
	r.status.Version = h.Version
	r.status.Workers = []node.WorkerStatus{{Name: "ide", State: "not running", Functions: []string{}}}
	h.SetHooks(Hooks{})
	h.startNodeCalls(t.Context())
	defer h.Close()
	waitWorkers := func(want string) {
		t.Helper()
		var got []byte
		for deadline := time.Now().Add(5 * time.Second); time.Now().Before(deadline); time.Sleep(50 * time.Millisecond) {
			list, _ := h.ListNodes(t.Context(), struct{}{})
			if got, _ = json.Marshal(list[0].Workers); string(got) == want {
				return
			}
		}
		t.Fatalf("workers %s, want %s", got, want)
	}
	waitWorkers(`[{"name":"ide","state":"not running","functions":[]}]`)
	// The bridge starts sharing ide over its open connection: no gateway auth, no reconnect.
	r.mu.Lock()
	r.status.Workers = []node.WorkerStatus{{Name: "ide", State: "shared", Functions: []string{"coder::read-file"}}}
	r.mu.Unlock()
	h.onFunctionsChange() // what the hub engine's function change event does
	waitWorkers(`[{"name":"ide","state":"shared","functions":["laptop::coder::read-file"]}]`)
}

// readingHub is nodeCallHub with its node reads running, after the two reads its join starts: the first catalog pass and the pass the node's version starts.
// Its engine lists no functions: a failed catalog read would read the node again on its retry.
func readingHub(t *testing.T) (*Hub, *nodeCallRPC) {
	t.Helper()
	h, r := nodeCallHub(t)
	h.engine, h.Sessions = &fakeEngine{}, &Sessions{}
	r.status.Version = h.Version
	h.SetHooks(Hooks{})
	h.startNodeCalls(t.Context())
	t.Cleanup(func() {
		h.Close()
		<-h.nodeReconciler.done // no read is left to race a restored variable
	})
	within(t, "the join's reads did not end", func() bool { return r.statusCalls() >= 2 && idle(h) })
	return h, r
}
func idle(h *Hub) bool {
	h.nodeReconciler.mu.Lock()
	defer h.nodeReconciler.mu.Unlock()
	return len(h.nodeReconciler.tasks) == 0
}
func shows(h *Hub, path, workers string) bool {
	h.mu.RLock()
	defer h.mu.RUnlock()
	w, _ := json.Marshal(h.nodeDetails["laptop"].Workers)
	return h.nodeDetails["laptop"].Path == path && string(w) == workers
}

// A node's path and a worker it shares with no functions change with no event the hub observes: the node says so.
func TestANodeThatSaysItChangedIsReadAgain(t *testing.T) {
	h, r := readingHub(t)
	changes := func(path string, workers ...node.WorkerStatus) {
		r.mu.Lock()
		r.status.Peers, r.status.Workers = []meshnet.PeerStatus{{Name: "hub", Path: path}}, append([]node.WorkerStatus{}, workers...)
		r.mu.Unlock()
	}
	changes("direct 192.0.2.1:41641", node.WorkerStatus{Name: "ide", State: "not running"})
	session := SessionContext{MeshNode: "laptop", SessionKey: h.Sessions.Open("laptop")}
	if out, err := h.middleware(t.Context(), nil, MiddlewareRequest{FunctionID: "mesh::node::changed", Namespace: "project", Context: session}); err != nil || string(out) != "null" {
		t.Fatalf("answer %s %v", out, err)
	}
	within(t, "the node was not read again", func() bool {
		return shows(h, "direct 192.0.2.1:41641", `[{"name":"ide","state":"not running","functions":[]}]`)
	})
	changes("relay hub")
	time.Sleep(300 * time.Millisecond)
	if !shows(h, "direct 192.0.2.1:41641", `[{"name":"ide","state":"not running","functions":[]}]`) {
		t.Fatal("the node was read with no event")
	}
}

// Listing the nodes reads each online node again, and the read's status event brings what moved to the page.
func TestListingTheNodesReadsThemAgain(t *testing.T) {
	h, r := readingHub(t)
	var pushes atomic.Int32
	h.mu.Lock()
	h.emit = func([]byte) { pushes.Add(1) }
	h.mu.Unlock()
	r.mu.Lock()
	r.status.Peers = []meshnet.PeerStatus{{Name: "hub", Path: "direct 192.0.2.1:41641"}}
	r.mu.Unlock()
	if _, err := h.listAndRead(t.Context(), struct{}{}); err != nil {
		t.Fatal(err)
	}
	within(t, "a list did not read the node", func() bool { return shows(h, "direct 192.0.2.1:41641", `[]`) && pushes.Load() == 1 })
	// GetNode lists too, and reads only the node it names.
	before := r.statusCalls()
	if _, err := h.GetNode(t.Context(), NodeRequest{Name: "laptop"}); err != nil {
		t.Fatal(err)
	}
	time.Sleep(200 * time.Millisecond)
	if got := r.statusCalls() - before; got != 1 || pushes.Load() != 1 {
		t.Fatalf("GetNode made %d status calls and %d status events, want 1 and none", got, pushes.Load()-1)
	}
}

// A version mismatch and a node function not registered yet each end with an event that queues the node: neither is retried.
func TestFailuresAnEventEndsAreNotRetried(t *testing.T) {
	old := retryMin
	t.Cleanup(func() { retryMin = old })
	retryMin = 5 * time.Millisecond
	for _, c := range []struct {
		name  string
		reads int // a version that moved also reads the catalog, which queues the node once more
		fail  func(*nodeCallRPC)
	}{
		{"another version", 2, func(r *nodeCallRPC) { r.status.Version = "0.1.0" }},
		{"a node function not registered yet", 1, func(r *nodeCallRPC) { r.statusErr = &iii.InvocationError{Code: "function_not_found"} }},
	} {
		t.Run(c.name, func(t *testing.T) {
			h, r := readingHub(t)
			r.mu.Lock()
			c.fail(r)
			r.mu.Unlock()
			before := r.statusCalls()
			h.queueNode("laptop")
			within(t, "the node was not read", func() bool { return r.statusCalls()-before >= c.reads })
			time.Sleep(200 * time.Millisecond)
			if got := r.statusCalls() - before; got != c.reads {
				t.Fatalf("%d reads with no event, want %d", got, c.reads)
			}
			h.queueNode("laptop")
			within(t, "an event did not read the node again", func() bool { return r.statusCalls()-before == c.reads+1 })
		})
	}
}

// A node whose read timed out announces nothing when it answers again: the read is retried at doubling waits, and any event of the node runs it at once.
func TestAFailingNodeBacksOffAndAnEventRetriesAtOnce(t *testing.T) {
	old := retryMin
	t.Cleanup(func() { retryMin = old })
	retryMin = 25 * time.Millisecond // large next to a busy -race run's scheduling delays
	h, r := readingHub(t)
	fail := func(err error) {
		r.mu.Lock()
		r.statusErr = err
		r.mu.Unlock()
	}
	fail(iii.ErrTimeout)
	before := r.statusCalls()
	h.queueNode("laptop")
	time.Sleep(time.Second)
	// Reads at 0, 25, 75, 175, 375 and 775 ms; a fixed 25 ms wait reads 40 times, and a busy machine only reads fewer.
	if got := r.statusCalls() - before; got > 8 {
		t.Fatalf("%d reads in 1 s", got)
	}
	within(t, "the failing node was not read again", func() bool { return r.statusCalls()-before >= 3 })
	// An event starts the waits over: the read after it follows 25 ms later, not 1.6 s.
	before = r.statusCalls()
	h.queueNode("laptop")
	time.Sleep(750 * time.Millisecond)
	if got := r.statusCalls() - before; got < 2 {
		t.Fatalf("%d reads in 750 ms after an event", got)
	}
	fail(nil)
	within(t, "the node that answers again was not read", func() bool { return idle(h) })
	retryMin = time.Minute // the task that read it has ended
	fail(iii.ErrTimeout)
	before = r.statusCalls()
	h.queueNode("laptop")
	within(t, "the node was not read", func() bool { return r.statusCalls()-before == 1 })
	// The event cuts the wait; an event during that read is read by the next pass, and nothing reads it again.
	r.mu.Lock()
	r.statusErr, r.onStatus = nil, func() { r.statusErr = iii.ErrTimeout; h.queueNode("laptop") }
	r.mu.Unlock()
	h.queueNode("laptop")
	within(t, "an event did not cut the wait", func() bool { return r.statusCalls()-before >= 3 })
	time.Sleep(100 * time.Millisecond)
	if got := r.statusCalls() - before; got != 3 {
		t.Fatalf("%d reads, want 3: one event was read twice", got)
	}
}
func TestApplyOnReconnect(t *testing.T) {
	h, r := nodeCallHub(t)
	r.status.Version = h.Version
	if err := h.OnNodeOnline(t.Context(), "laptop"); err != nil {
		t.Fatal(err)
	}
	if r.workers.Namespace != "project" {
		t.Fatalf("stale apply %+v", r.workers)
	}
	r.calls = nil
	r.fail = "mesh-node::workers::apply"
	if err := h.OnNodeOnline(t.Context(), "laptop"); err == nil || !reflect.DeepEqual(r.calls, []string{"workers::apply"}) {
		t.Fatalf("continued after failed apply %v %v", err, r.calls)
	}
}

// The catalog follows the namespace, in its own call, only when the node holds another revision.
func TestCatalogPushedWhenTheRevisionDiffers(t *testing.T) {
	h, r := nodeCallHub(t)
	r.status.Version = h.Version
	online := func(want ...string) error {
		t.Helper()
		r.calls = nil
		err := h.OnNodeOnline(t.Context(), "laptop")
		if !reflect.DeepEqual(r.calls, want) {
			t.Fatalf("calls %v, want %v", r.calls, want)
		}
		return err
	}
	if err := online("workers::apply"); err != nil {
		t.Fatal("before the first read: ", err)
	}
	catalog := node.Catalog{Rev: "r1", Functions: []node.CatalogEntry{{ID: "shell::exec", Owner: "hub", Description: "run"}}}
	h.shared = shared{catalog: catalog, rows: map[string]sharedRow{"shell::exec": {"hub", "shell"}}}
	if err := online("workers::apply", "catalog::apply"); err != nil || !reflect.DeepEqual(*r.catalog, catalog) {
		t.Fatalf("catalog %+v: %v", r.catalog, err)
	}
	r.status.Catalog = "r1"
	if err := online("workers::apply"); err != nil {
		t.Fatal(err)
	}
	// A hub restarted on another namespace: the apply clears the revision the status reported.
	h.Namespace = "other"
	if err := online("workers::apply", "catalog::apply"); err != nil {
		t.Fatal(err)
	}
	h.Namespace = "project"
	h.shared.catalog.Rev = "r2"
	r.fail, r.workers = "mesh-node::catalog::apply", node.WorkersRequest{}
	if err := online("workers::apply", "catalog::apply"); err == nil || r.workers.Namespace != "project" {
		t.Fatalf("a failed catalog call: %v, namespace %q", err, r.workers.Namespace)
	}
	r.fail, r.status.Catalog = "", "r1"
	h.shared.catalog = node.Catalog{Functions: []node.CatalogEntry{}}
	if err := online("workers::apply", "catalog::apply"); err != nil || r.catalog.Rev != "" || r.catalog.Functions == nil || len(r.catalog.Functions) != 0 {
		t.Fatalf("an empty catalog: %+v %v", r.catalog, err)
	}
}

// A machine that comes online after the reconciler started is read on its online event, not by polling.
func TestNodeGoingOnlineIsReconciledByItsEvent(t *testing.T) {
	h, r, up := idleNodeCallHub(t)
	r.status.Version = h.Version
	r.status.Workers = []node.WorkerStatus{{Name: "ide", State: "shared", Functions: []string{"coder::read-file"}}}
	h.SetHooks(Hooks{})
	h.startNodeCalls(t.Context())
	defer h.Close()
	up()
	for deadline := time.Now().Add(5 * time.Second); ; time.Sleep(20 * time.Millisecond) {
		list, _ := h.ListNodes(t.Context(), struct{}{})
		if len(list) == 1 && len(list[0].Workers) == 1 {
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("online node not reconciled: %+v", list)
		}
	}
	// A node on another version waits for an event, and its removal ends the wait.
	r.mu.Lock()
	r.status.Version = "0.0.9"
	r.mu.Unlock()
	h.queueNode("laptop")
	if err := h.Coordinator.Remove("laptop"); err != nil {
		t.Fatal(err)
	}
	h.nodeReconciler.mu.Lock()
	defer h.nodeReconciler.mu.Unlock()
	if len(h.nodeReconciler.tasks) != 0 {
		t.Fatal("a removed machine is still re-read")
	}
}

// A rename drops everything the hub keyed by the old name before the machine hears its new one.
func TestRenameFollowsOnTheHub(t *testing.T) {
	c, err := coordinator.New(t.TempDir(), func(string, string) (string, error) { return "", nil })
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { c.Close() })
	mux := http.NewServeMux()
	c.Handle(mux)
	front := httptest.NewServer(mux)
	t.Cleanup(front.Close)
	h := &Hub{Coordinator: c, Namespace: "project", Version: "0.1.0", config: DefaultConfig(), Sessions: &Sessions{}, Tickets: gateway.NewTickets(10*time.Second, time.Now)}
	h.MeshGateway = gateway.New(c.Node, "ws://127.0.0.1:1", h.Tickets)
	defer h.MeshGateway.Close()
	revoked := make(chan string, 4)
	h.SetHooks(Hooks{RevokeNode: func(name string) { revoked <- name }})
	c.SetGone(h.revoke)
	m, err := meshnet.UpUserspace(t.TempDir(), front.URL, "secret", "laptop", c.Public())
	if err != nil {
		t.Fatal(err)
	}
	defer m.Close()
	named := func(want string) {
		t.Helper()
		for deadline := time.Now().Add(10 * time.Second); ; time.Sleep(10 * time.Millisecond) {
			if got, _ := m.SelfName(); got == want {
				return
			} else if time.Now().After(deadline) {
				t.Fatalf("self name %q, want %q", got, want)
			}
		}
	}
	named("laptop")
	session := h.Sessions.Open("laptop")
	h.mu.Lock()
	h.nodeDetails = map[string]NodeDetails{"laptop": {NodeEntry: NodeEntry{Name: "laptop", Workers: []ComponentState{{Name: "ide", State: "shared", Functions: []string{"laptop::coder::read-file"}}}}}}
	h.mu.Unlock()
	if name, err := m.Rename(t.Context(), "work"); err != nil || name != "work" {
		t.Fatalf("rename: %q %v", name, err)
	}
	select {
	case old := <-revoked:
		if old != "laptop" {
			t.Fatalf("revoked %q", old)
		}
	default:
		t.Fatal("the hub answered before it dropped the old name")
	}
	h.mu.RLock()
	_, cached := h.nodeDetails["laptop"]
	h.mu.RUnlock()
	if _, ok := h.Sessions.Valid(session); ok || cached {
		t.Fatalf("old name kept: session %v, cached details %v", ok, cached)
	}
	if list, _ := h.ListNodes(t.Context(), struct{}{}); len(list) != 1 || list[0].Name != "work" || len(list[0].Workers) != 0 {
		t.Fatalf("nodes %+v", list)
	}
	named("work")
}

// The page re-reads mesh::status on these pushes instead of polling it.
func TestStatusChangesArePublished(t *testing.T) {
	var mu sync.Mutex
	var published []StatusResult
	record := func(h *Hub) {
		h.emit = func(b []byte) {
			var s StatusResult
			if err := json.Unmarshal(b, &s); err != nil {
				t.Error(err)
			}
			mu.Lock()
			published = append(published, s)
			mu.Unlock()
		}
	}
	take := func(what string, n int) []StatusResult {
		t.Helper()
		mu.Lock()
		defer mu.Unlock()
		p := published
		published = nil
		if len(p) != n {
			t.Fatalf("%s: %d pushes, want %d: %+v", what, len(p), n, p)
		}
		return p
	}
	ctx := t.Context()

	h, f := tunnelHub(t, true)
	record(h)
	h.reconcileTunnel(ctx)
	if p := take("tunnel starting", 1); p[0].Tunnel == nil || p[0].Tunnel.State != "starting" {
		t.Fatalf("%+v", p[0])
	}
	h.reconcileTunnel(ctx)
	take("the same tunnel state", 0)
	f.state, f.url = "ready", "https://abc234.opentunnel.xyz"
	h.reconcileTunnel(ctx)
	if p := take("tunnel ready", 1); p[0].PublicURL != f.url {
		t.Fatalf("%+v", p[0])
	}
	f.statusErr = iii.ErrTimeout // the lease still holds, so the URL stays
	h.reconcileTunnel(ctx)
	h.reconcileTunnel(ctx)
	if p := take("a failure under a live lease", 1); p[0].PublicURL != f.url || p[0].Tunnel.Error != tunnelNotResponding {
		t.Fatalf("%+v %+v", p[0], p[0].Tunnel)
	}

	hc, _ := tunnelHub(t, true)
	hc.ConfigID = "project_mesh"
	record(hc)
	if _, err := hc.onConfigChange(ctx, &configRPC{t: t, value: json.RawMessage(`{"value":{"public_url":"https://hub.example"}}`)}, ConfigChangeEvent{ID: "project_mesh"}); err != nil {
		t.Fatal(err)
	}
	if p := take("an applied configuration", 1); p[0].PublicURL != "https://hub.example" {
		t.Fatalf("%+v", p[0])
	}

	hn, r := nodeCallHub(t)
	record(hn)
	r.status.Version = hn.Version
	if err := hn.OnNodeOnline(ctx, "laptop"); err != nil {
		t.Fatal(err)
	}
	take("a new node entry", 1)
	if err := hn.OnNodeOnline(ctx, "laptop"); err != nil {
		t.Fatal(err)
	}
	take("the same node entry", 0)
	r.status.Workers = []node.WorkerStatus{{Name: "ide", State: "shared", Functions: []string{"coder::read-file"}}}
	if err := hn.OnNodeOnline(ctx, "laptop"); err != nil {
		t.Fatal(err)
	}
	take("a node sharing a worker", 1)
}
