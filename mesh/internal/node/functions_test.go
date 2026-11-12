package node

import (
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"encoding/json"
	"errors"
	"net/netip"
	"reflect"
	"runtime"
	"sync"
	"testing"
	"time"

	"github.com/iii-hq/workers/mesh/internal/meshnet"
)

type testNetwork struct {
	mu      sync.Mutex
	port    uint16
	name    string
	changes chan struct{}
	removed chan struct{}
}

func (m *testNetwork) Self() netip.Addr { return netip.MustParseAddr("fd00::2") }
func (m *testNetwork) SelfName() (string, bool) {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.name, m.name != ""
}
func (m *testNetwork) Peers() []meshnet.PeerStatus {
	return []meshnet.PeerStatus{{Name: "hub", Path: "relay hub"}}
}
func (m *testNetwork) GatewayPort() (uint16, bool) {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.port, m.port != 0
}
func (m *testNetwork) GatewayChanges() <-chan struct{} { return m.changes }
func (m *testNetwork) Removed() <-chan struct{}        { return m.removed }
func (m *testNetwork) setPort(p uint16) {
	m.mu.Lock()
	m.port = p
	m.mu.Unlock()
	m.changes <- struct{}{}
}

type testWorkers struct {
	mu       sync.Mutex
	runtime  Runtime
	runtimes chan Runtime
	catalog  *Catalog
}

func (w *testWorkers) SetRuntime(_ context.Context, r Runtime) error {
	w.mu.Lock()
	w.runtime = r
	w.mu.Unlock()
	if w.runtimes != nil {
		w.runtimes <- r
	}
	return nil
}
func (w *testWorkers) SetCatalog(c Catalog) {
	w.mu.Lock()
	w.catalog = &c
	w.mu.Unlock()
}
func (w *testWorkers) CatalogRev() string {
	w.mu.Lock()
	defer w.mu.Unlock()
	if w.catalog == nil {
		return ""
	}
	return w.catalog.Rev
}
func (w *testWorkers) Status() []WorkerStatus {
	return []WorkerStatus{{Name: "shell", State: "shared", Functions: []string{"shell::exec"}}}
}
func testNode(t *testing.T) (*Node, ed25519.PrivateKey, *testNetwork, *testWorkers) {
	t.Helper()
	return testNodeWith(t, Options{})
}
func testNodeWith(t *testing.T, o Options) (*Node, ed25519.PrivateKey, *testNetwork, *testWorkers) {
	t.Helper()
	pub, k, _ := ed25519.GenerateKey(rand.Reader)
	m := &testNetwork{port: 50222, name: "laptop-2", changes: make(chan struct{}, 8), removed: make(chan struct{})}
	w := &testWorkers{}
	o.Mesh, o.HubIdentity, o.Version, o.Workers = m, pub, "0.1.0", w
	if o.URL == "" {
		o.URL = "ws://gateway.invalid"
	}
	n, err := New(o)
	if err != nil {
		t.Fatal(err)
	}
	return n, k, m, w
}
func TestVerifyReportsSkew(t *testing.T) {
	n, k, _, _ := testNode(t)
	now := time.Unix(1700000000, 0)
	s, _ := Sign(k, "mesh-node::status", struct{}{}, now.Add(90*time.Second))
	if _, err := n.verifier.Verify("mesh-node::status", s, now); !errors.Is(err, ErrSkew) {
		t.Fatal(err)
	}
	if n.verifier.LastSkew() != 90*time.Second {
		t.Fatal(n.verifier.LastSkew())
	}
	s, _ = Sign(k, "mesh-node::status", struct{}{}, time.Now())
	raw, err := n.handle(t.Context(), "mesh-node::status", s)
	if err != nil {
		t.Fatal(err)
	}
	var status Status
	json.Unmarshal(raw, &status)
	if status.ClockSkewSecs != 90 || status.Name != "laptop-2" || status.OS != runtime.GOOS || !reflect.DeepEqual(status.Workers, []WorkerStatus{{Name: "shell", State: "shared", Functions: []string{"shell::exec"}}}) {
		t.Fatalf("status %+v", status)
	}
}
func TestApplyUsesAssignedIdentityNamespaceAndGateway(t *testing.T) {
	n, k, _, w := testNode(t)
	w.catalog = &Catalog{Rev: "r1"}
	s, _ := Sign(k, "mesh-node::workers::apply", WorkersRequest{Namespace: "project"}, time.Now())
	raw, err := n.handle(t.Context(), "mesh-node::workers::apply", s)
	if err != nil {
		t.Fatal(err)
	}
	// The reply carries the revision held after the apply, which the hub compares.
	var reply Status
	json.Unmarshal(raw, &reply)
	want := Runtime{Node: "laptop-2", Namespace: "project", URL: "ws://gateway.invalid"}
	if w.runtime != want || reply.Catalog != "r1" {
		t.Fatalf("runtime %+v reply %s", w.runtime, raw)
	}
}
func TestUnsignedApplyCannotReachWorkers(t *testing.T) {
	n, _, _, w := testNode(t)
	if _, err := n.handle(t.Context(), "mesh-node::workers::apply", Signed{Payload: json.RawMessage(`{"namespace":"evil"}`)}); !errors.Is(err, ErrUnsigned) {
		t.Fatal(err)
	}
	if w.runtime != (Runtime{}) {
		t.Fatal("unsigned mutation reached workers")
	}
}
func TestApplyRequiresNamespaceAndLiveGateway(t *testing.T) {
	for _, missing := range []string{"namespace", "gateway", "name"} {
		t.Run(missing, func(t *testing.T) {
			n, k, m, w := testNode(t)
			req := WorkersRequest{Namespace: "proj"}
			switch missing {
			case "namespace":
				req.Namespace = ""
			case "gateway":
				m.port = 0
			case "name":
				m.name = ""
			}
			s, _ := Sign(k, "mesh-node::workers::apply", req, time.Now())
			if _, err := n.handle(t.Context(), "mesh-node::workers::apply", s); err == nil {
				t.Fatal("incomplete identity accepted")
			}
			if w.runtime != (Runtime{}) {
				t.Fatal("runtime set")
			}
		})
	}
}

// The catalog reaches the workers only signed; the status reports the revision they hold.
func TestCatalogApply(t *testing.T) {
	n, k, _, w := testNode(t)
	catalog := Catalog{Rev: "r1", Functions: []CatalogEntry{{ID: "shell::exec", Owner: "hub", Description: "run", RequestFormat: json.RawMessage(`{"type":"object"}`)}}}
	if _, err := n.handle(t.Context(), "mesh-node::catalog::apply", Signed{Payload: json.RawMessage(`{"rev":"evil","functions":[]}`)}); !errors.Is(err, ErrUnsigned) || w.catalog != nil {
		t.Fatalf("unsigned catalog: %v %+v", err, w.catalog)
	}
	s, _ := Sign(k, "mesh-node::catalog::apply", catalog, time.Now())
	if raw, err := n.handle(t.Context(), "mesh-node::catalog::apply", s); err != nil || string(raw) != "{}" || !reflect.DeepEqual(*w.catalog, catalog) {
		t.Fatalf("catalog %+v reply %s: %v", w.catalog, raw, err)
	}
	if got := n.Status().Catalog; got != "r1" {
		t.Fatalf("status catalog %q", got)
	}
	// A hub on another version refuses every mirror call, so the mirrors go.
	s, _ = Sign(k, "mesh-node::status", StatusRequest{HubVersion: "0.9.0"}, time.Now())
	if _, err := n.handle(t.Context(), "mesh-node::status", s); err != nil || w.catalog.Rev != "" || len(w.catalog.Functions) != 0 {
		t.Fatalf("a hub on another version kept the catalog: %+v %v", w.catalog, err)
	}
	s, _ = Sign(k, "mesh-node::catalog::apply", catalog, time.Now())
	if _, err := n.handle(t.Context(), "mesh-node::catalog::apply", s); err != nil || w.catalog.Rev != "r1" {
		t.Fatalf("catalog %+v: %v", w.catalog, err)
	}
	s, _ = Sign(k, "mesh-node::catalog::apply", Catalog{Functions: []CatalogEntry{}}, time.Now())
	if _, err := n.handle(t.Context(), "mesh-node::catalog::apply", s); err != nil || w.catalog.Rev != "" || len(w.catalog.Functions) != 0 || n.Status().Catalog != "" {
		t.Fatalf("an empty catalog does not clear: %+v %v", w.catalog, err)
	}
}
