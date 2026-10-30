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
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/coordinator"
	"github.com/iii-hq/workers/mesh/internal/meshnet"
	"github.com/iii-hq/workers/mesh/internal/node"
	"github.com/iii-hq/workers/mesh/internal/registry"
	"tailscale.com/tsnet"
)

type nodeCallRPC struct {
	status   node.Status
	verifier *node.Verifier
	calls    []string
	workers  node.WorkersRequest
	drives   node.DrivesRequest
	update   node.UpdateRequest
	fail     string
}

func (r *nodeCallRPC) Trigger(_ context.Context, req iii.TriggerRequest) (json.RawMessage, error) {
	if req.Namespace != "default" || !strings.HasPrefix(req.FunctionID, "laptop::mesh-node::") {
		return nil, errors.New("incorrect control namespace or prefix")
	}
	fn := strings.TrimPrefix(req.FunctionID, "laptop::")
	var signed node.Signed
	var wire any
	json.Unmarshal(req.Data, &wire)
	b, _ := json.Marshal(wire)
	json.Unmarshal(b, &signed)
	payload, err := r.verifier.Verify(fn, signed, time.Now())
	if err != nil {
		return nil, err
	}
	if fn == "mesh-node::status" {
		return json.Marshal(r.status)
	}
	r.calls = append(r.calls, strings.TrimPrefix(fn, "mesh-node::"))
	if r.fail == fn {
		return nil, errors.New("apply failed")
	}
	switch fn {
	case "mesh-node::update":
		json.Unmarshal(payload, &r.update)
	case "mesh-node::workers::apply":
		json.Unmarshal(payload, &r.workers)
	case "mesh-node::drives::apply":
		json.Unmarshal(payload, &r.drives)
	}
	return json.RawMessage(`[]`), nil
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
	r := &nodeCallRPC{status: node.Status{Name: "laptop", OS: "linux", Arch: "amd64", Target: "x86_64-unknown-linux-musl", Version: "0.1.0", Operator: "alice", Workers: []node.WorkerStatus{}, Drives: []node.DriveStatus{}}, verifier: node.NewVerifier(c.Identity().Public().(ed25519.PublicKey), time.Minute)}
	h := &Hub{Coordinator: c, Namespace: "project", Version: "0.2.0", config: DefaultConfig(), nodeClient: r}
	h.resolveArtifact = func(ctx context.Context, worker, version, target string) (registry.Artifact, error) {
		if target != "x86_64-unknown-linux-musl" {
			t.Errorf("guessed target %q", target)
		}
		return registry.Artifact{Version: version, URL: "https://artifacts.example/" + worker, SHA256: strings.Repeat("a", 64)}, nil
	}
	return h, r
}
func TestUpdateFirstWhenVersionsDiffer(t *testing.T) {
	h, r := nodeCallHub(t)
	if err := h.OnNodeOnline(t.Context(), "laptop"); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(r.calls, []string{"update"}) || r.update.Version != "0.2.0" || r.update.ArtifactURL != "https://artifacts.example/mesh" {
		t.Fatalf("calls %v update %+v", r.calls, r.update)
	}
	if h.Status().Nodes.Outdated != 1 {
		t.Fatal("outdated node missing")
	}
	r.status.Version = "0.2.0"
	r.calls = nil
	if err := h.OnNodeOnline(t.Context(), "laptop"); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(r.calls, []string{"workers::apply", "drives::apply"}) {
		t.Fatal(r.calls)
	}
	if h.Status().Nodes.Outdated != 0 {
		t.Fatal("updated node still outdated")
	}
	list, _ := h.ListNodes(t.Context(), struct{}{})
	if len(list) != 1 || list[0].Version != "0.2.0" || list[0].OS != "linux" || list[0].Arch != "amd64" {
		t.Fatalf("list %+v", list)
	}
	info := h.Coordinator.Nodes()[0]
	if info.Operator != "alice" || info.Target != "x86_64-unknown-linux-musl" {
		t.Fatalf("metadata %+v", info)
	}
}
func TestApplyOnReconnect(t *testing.T) {
	h, r := nodeCallHub(t)
	r.status.Version = h.Version
	h.config.Nodes["laptop"] = NodeConfig{Workers: []string{"old"}}
	h.config.Nodes["laptop"] = NodeConfig{Workers: []string{"ide@^0.12", "shell"}}
	h.config.Drives["work"] = DriveConfig{Server: "laptop", Nodes: []string{"*"}}
	h.applyCoordinator(h.config)
	if err := h.OnNodeOnline(t.Context(), "laptop"); err != nil {
		t.Fatal(err)
	}
	if len(r.workers.Workers) != 2 || r.workers.Workers[0].Name != "ide" || r.workers.Workers[0].Version != "^0.12" || r.workers.Workers[1].Name != "shell" || r.workers.Namespace != "project" {
		t.Fatalf("stale apply %+v", r.workers)
	}
	if len(r.drives.Mount) != 1 || r.drives.Mount[0].Name != "work" || r.drives.Mount[0].Server != "laptop" {
		t.Fatalf("drives %+v", r.drives)
	}
	r.calls = nil
	r.fail = "mesh-node::workers::apply"
	if err := h.OnNodeOnline(t.Context(), "laptop"); err == nil || !reflect.DeepEqual(r.calls, []string{"workers::apply"}) {
		t.Fatalf("continued after failed apply %v %v", err, r.calls)
	}
	r.fail = ""
	r.calls = nil
	r.status.Target = ""
	if err := h.OnNodeOnline(t.Context(), "laptop"); !errors.Is(err, node.ErrUnsupportedTarget) || len(r.calls) != 0 {
		t.Fatalf("unsupported target: %v calls %v", err, r.calls)
	}
}

func TestHubDesiredDrivesConfigReplacementAndFailure(t *testing.T) {
	h, r := nodeCallHub(t)
	r.status.Version = h.Version
	if _, err := h.fetchNodeStatus(t.Context(), "laptop"); err != nil {
		t.Fatal(err)
	}
	cfg := h.Config()
	cfg.Drives = map[string]DriveConfig{"proj": {Server: "laptop", Nodes: []string{"laptop"}}}
	if err := h.applyConfig(t.Context(), cfg); err != nil {
		t.Fatal(err)
	}
	if err := h.OnNodeOnline(t.Context(), "laptop"); err != nil {
		t.Fatal(err)
	}
	if len(r.drives.Serve) != 1 || len(r.drives.Mount) != 1 || !r.drives.Prefix.IsValid() || !r.drives.Mount[0].Addr.IsValid() {
		t.Fatalf("signed request dropped authority: %+v", r.drives)
	}
	cfg.Drives = nil
	if err := h.applyConfig(t.Context(), cfg); err != nil {
		t.Fatal(err)
	}
	if err := h.OnNodeOnline(t.Context(), "laptop"); err != nil {
		t.Fatal(err)
	}
	if len(r.drives.Serve)+len(r.drives.Mount) != 0 {
		t.Fatal("removed config retained drive")
	}
}
