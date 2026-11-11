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
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/coordinator"
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
		return json.Marshal(r.status)
	}
	r.calls = append(r.calls, strings.TrimPrefix(fn, "mesh-node::"))
	if r.fail == fn {
		return nil, errors.New("apply failed")
	}
	json.Unmarshal(payload, &r.workers)
	return json.RawMessage(`{}`), nil
}
func nodeCallHub(t *testing.T) (*Hub, *nodeCallRPC) {
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
	ctx, cancel := context.WithTimeout(t.Context(), 10*time.Second)
	defer cancel()
	if _, err = peer.Up(ctx); err != nil {
		t.Fatal(err)
	}
	r := &nodeCallRPC{status: node.Status{Name: "laptop", OS: "linux", Arch: "amd64", Version: "0.1.0", Workers: []node.WorkerStatus{}}, verifier: node.NewVerifier(c.Identity().Public().(ed25519.PublicKey), time.Minute)}
	h := &Hub{Coordinator: c, Namespace: "project", Version: "0.2.0", config: DefaultConfig(), nodeClient: r}
	return h, r
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
	defer func(n int) { nodeRefreshTicks = n }(nodeRefreshTicks)
	nodeRefreshTicks = 1
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
	waitWorkers(`[{"name":"ide","state":"shared","functions":["laptop::coder::read-file"]}]`)
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
