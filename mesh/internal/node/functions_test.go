package node

import (
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"net/netip"
	"os"
	"path/filepath"
	"reflect"
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
	workers  []Worker
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
func (w *testWorkers) Apply(_ context.Context, workers []Worker) ([]WorkerResult, error) {
	w.mu.Lock()
	defer w.mu.Unlock()
	w.workers = workers
	return []WorkerResult{}, nil
}
func (w *testWorkers) Status() []WorkerStatus {
	w.mu.Lock()
	defer w.mu.Unlock()
	statuses := []WorkerStatus{}
	for _, worker := range w.workers {
		statuses = append(statuses, WorkerStatus{Name: worker.Name, Version: worker.Version, State: "running"})
	}
	return statuses
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
	o.Mesh, o.HubIdentity, o.Version, o.Operator, o.Workers = m, pub, "0.1.0", "alice", w
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
	if status.ClockSkewSecs != 90 || status.Name != "laptop-2" || status.Operator != "alice" || status.Target == "" {
		t.Fatalf("status %+v", status)
	}
}
func TestApplyUsesAssignedIdentityNamespaceAndGateway(t *testing.T) {
	n, k, _, w := testNode(t)
	req := WorkersRequest{Namespace: "project", Workers: []Worker{{Name: "ide", Version: "0.12", ArtifactURL: "https://example.com/ide", SHA256: "digest"}}}
	s, _ := Sign(k, "mesh-node::workers::apply", req, time.Now())
	if _, err := n.handle(t.Context(), "mesh-node::workers::apply", s); err != nil {
		t.Fatal(err)
	}
	want := Runtime{Node: "laptop-2", Namespace: "project", URL: "ws://hub.iii.internal:50222"}
	if w.runtime != want || !reflect.DeepEqual(w.workers, req.Workers) {
		t.Fatalf("runtime %+v workers %+v", w.runtime, w.workers)
	}
}
func TestUnsignedApplyCannotReachSupervisor(t *testing.T) {
	n, _, _, w := testNode(t)
	if _, err := n.handle(t.Context(), "mesh-node::workers::apply", Signed{Payload: json.RawMessage(`{"namespace":"evil","workers":[]}`)}); !errors.Is(err, ErrUnsigned) {
		t.Fatal(err)
	}
	if w.runtime != (Runtime{}) || w.workers != nil {
		t.Fatal("unsigned mutation reached workers")
	}
}
func TestApplyRequiresNamespaceAndLiveGateway(t *testing.T) {
	for _, missing := range []string{"namespace", "gateway", "name"} {
		t.Run(missing, func(t *testing.T) {
			n, k, m, w := testNode(t)
			req := WorkersRequest{Namespace: "proj", Workers: []Worker{}}
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
			if w.workers != nil {
				t.Fatal("workers applied")
			}
		})
	}
}

func TestSignedUpdateRequestsServiceRestart(t *testing.T) {
	n, k, m, _ := testNode(t)
	m.port = 0
	binary := filepath.Join(t.TempDir(), "mesh")
	os.WriteFile(binary, []byte("old"), 0755)
	content := []byte("replacement")
	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { w.Write(content) }))
	defer server.Close()
	n.updater = &Updater{BinaryPath: binary, Client: server.Client()}
	s, _ := Sign(k, "mesh-node::update", UpdateRequest{Version: "0.2.0", ArtifactURL: server.URL, SHA256: fmt.Sprintf("%x", sha256.Sum256(content))}, time.Now())
	if _, err := n.handle(t.Context(), "mesh-node::update", s); err != nil {
		t.Fatal(err)
	}
	if err := n.Run(t.Context()); !errors.Is(err, ErrRestartRequired) {
		t.Fatalf("did not request service restart: %v", err)
	}
}

func TestApplyResolutionErrorsRemainVisibleAndClear(t *testing.T) {
	n, k, _, w := testNode(t)
	apply := func(payload string) {
		t.Helper()
		signed, err := Sign(k, "mesh-node::workers::apply", json.RawMessage(payload), time.Now())
		if err != nil {
			t.Fatal(err)
		}
		if _, err := n.handle(t.Context(), "mesh-node::workers::apply", signed); err != nil {
			t.Fatal(err)
		}
	}
	apply(`{"namespace":"project","workers":[{"name":"old","version":"1","artifact_url":"https://example.com/old","sha256":"old-digest"}]}`)
	apply(`{"namespace":"project","workers":[{"name":"valid","version":"2","artifact_url":"https://example.com/valid","sha256":"valid-digest"},{"name":"unsupported","resolution_error":"worker has no binary for target","artifact_url":"https://example.com/must-not-run"}]}`)
	if len(w.workers) != 1 || w.workers[0].Name != "valid" {
		t.Fatalf("controller must receive only complete runnable desired set, got %+v", w.workers)
	}
	raw, err := json.Marshal(n.Status())
	if err != nil {
		t.Fatal(err)
	}
	var status struct {
		Workers []struct{ Name, Version, State, Error string }
	}
	if err = json.Unmarshal(raw, &status); err != nil {
		t.Fatal(err)
	}
	if len(status.Workers) != 2 {
		t.Fatalf("lost desired worker: %s", raw)
	}
	found := map[string]struct{ Name, Version, State, Error string }{}
	for _, worker := range status.Workers {
		found[worker.Name] = worker
	}
	if found["valid"].State != "running" || found["unsupported"].State != "error" || found["unsupported"].Error != "worker has no binary for target" || found["unsupported"].Version != "" {
		t.Fatalf("status %s", raw)
	}
	apply(`{"namespace":"project","workers":[{"name":"unsupported","version":"3","artifact_url":"https://example.com/recovered","sha256":"recovered-digest"}]}`)
	raw, _ = json.Marshal(n.Status())
	json.Unmarshal(raw, &status)
	if len(status.Workers) != 1 || status.Workers[0].Name != "unsupported" || status.Workers[0].State != "running" || status.Workers[0].Error != "" {
		t.Fatalf("stale resolution error after recovery: %s", raw)
	}
	apply(`{"namespace":"project","workers":[{"name":"unsupported","resolution_error":"another resolution failure"}]}`)
	if len(w.workers) != 0 {
		t.Fatalf("previously runnable failed worker retained: %+v", w.workers)
	}
	apply(`{"namespace":"project","workers":[]}`)
	if len(n.Status().Workers) != 0 {
		t.Fatalf("removed error retained: %+v", n.Status().Workers)
	}
}
