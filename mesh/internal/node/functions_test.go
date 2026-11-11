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
	s, _ := Sign(k, "mesh-node::workers::apply", WorkersRequest{Namespace: "project"}, time.Now())
	raw, err := n.handle(t.Context(), "mesh-node::workers::apply", s)
	if err != nil {
		t.Fatal(err)
	}
	want := Runtime{Node: "laptop-2", Namespace: "project", URL: "ws://gateway.invalid"}
	if w.runtime != want || string(raw) != "{}" {
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
