package hub

import (
	"context"
	"crypto/ed25519"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"net/netip"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/coder/websocket"
	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/meshnet"
	"github.com/iii-hq/workers/mesh/internal/node"
	"github.com/iii-hq/workers/mesh/internal/registry"
)

type signedNetwork struct{}

func (signedNetwork) Self() netip.Addr         { return netip.MustParseAddr("fd00::2") }
func (signedNetwork) SelfName() (string, bool) { return "laptop", true }
func (signedNetwork) Peers() []meshnet.PeerStatus {
	return []meshnet.PeerStatus{{Name: "hub", Path: "relay hub"}}
}
func (signedNetwork) GatewayPort() (uint16, bool)     { return 50222, true }
func (signedNetwork) GatewayChanges() <-chan struct{} { return nil }
func (signedNetwork) Removed() <-chan struct{}        { return nil }

type signedWorkers struct {
	mu               sync.Mutex
	runtime          node.Runtime
	applied          chan node.WorkersRequest
	workers          []node.Worker
	started, stopped []string
}

func (w *signedWorkers) SetRuntime(_ context.Context, r node.Runtime) error {
	w.mu.Lock()
	defer w.mu.Unlock()
	w.runtime = r
	return nil
}
func (w *signedWorkers) Apply(_ context.Context, workers []node.Worker) ([]node.WorkerResult, error) {
	w.mu.Lock()
	namespace := w.runtime.Namespace
	oldNames, nextNames := map[string]bool{}, map[string]bool{}
	for _, worker := range w.workers {
		oldNames[worker.Name] = true
	}
	for _, worker := range workers {
		nextNames[worker.Name] = true
		if !oldNames[worker.Name] {
			w.started = append(w.started, worker.Name)
		}
	}
	for _, worker := range w.workers {
		if !nextNames[worker.Name] {
			w.stopped = append(w.stopped, worker.Name)
		}
	}
	w.workers = append([]node.Worker{}, workers...)
	w.mu.Unlock()
	if w.applied != nil {
		w.applied <- node.WorkersRequest{Namespace: namespace, Workers: workers}
	}
	return []node.WorkerResult{}, nil
}
func (w *signedWorkers) Status() []node.WorkerStatus {
	w.mu.Lock()
	defer w.mu.Unlock()
	statuses := []node.WorkerStatus{}
	for _, worker := range w.workers {
		statuses = append(statuses, node.WorkerStatus{Name: worker.Name, Version: worker.Version, State: "running"})
	}
	return statuses
}

type registeredNodeRPC struct {
	mu         sync.Mutex
	conn       *websocket.Conn
	registered map[string]bool
	replies    map[string]chan map[string]json.RawMessage
	next       int
	notReady   chan struct{}
	namespace  string
}

func (r *registeredNodeRPC) serve(ctx context.Context, w http.ResponseWriter, request *http.Request) {
	c, err := websocket.Accept(w, request, nil)
	if err != nil {
		return
	}
	defer c.CloseNow()
	r.mu.Lock()
	r.conn = c
	r.mu.Unlock()
	for {
		_, raw, err := c.Read(ctx)
		if err != nil {
			return
		}
		var frame map[string]json.RawMessage
		json.Unmarshal(raw, &frame)
		var kind, id string
		json.Unmarshal(frame["type"], &kind)
		r.mu.Lock()
		switch kind {
		case "registerfunction":
			json.Unmarshal(frame["id"], &id)
			r.registered[id] = true
		case "invokefunction":
			if string(frame["function_id"]) == `"engine::workers::register"` {
				var data map[string]string
				json.Unmarshal(frame["data"], &data)
				r.namespace = data["namespace"]
			}
		case "invocationresult":
			json.Unmarshal(frame["invocation_id"], &id)
			if ch := r.replies[id]; ch != nil {
				ch <- frame
				delete(r.replies, id)
			}
		}
		r.mu.Unlock()
	}
}
func (r *registeredNodeRPC) Trigger(ctx context.Context, req iii.TriggerRequest) (json.RawMessage, error) {
	if req.Namespace != "default" {
		return nil, fmt.Errorf("wrong control namespace %s", req.Namespace)
	}
	fn := strings.TrimPrefix(req.FunctionID, "laptop::")
	if fn == req.FunctionID {
		return nil, errors.New("node prefix missing")
	}
	r.mu.Lock()
	if !r.registered[fn] || r.namespace != "default" {
		r.mu.Unlock()
		select {
		case r.notReady <- struct{}{}:
		default:
		}
		return nil, errors.New("node handler is not registered")
	}
	r.next++
	id := fmt.Sprintf("00000000-0000-4000-8000-%012d", r.next)
	response := make(chan map[string]json.RawMessage, 1)
	r.replies[id] = response
	c := r.conn
	r.mu.Unlock()
	defer func() { r.mu.Lock(); delete(r.replies, id); r.mu.Unlock() }()
	var payload any
	json.Unmarshal(req.Data, &payload)
	raw, _ := json.Marshal(map[string]any{"type": "invokefunction", "invocation_id": id, "function_id": fn, "data": payload})
	if err := c.Write(ctx, websocket.MessageText, raw); err != nil {
		return nil, err
	}
	select {
	case frame := <-response:
		if string(frame["error"]) != "" && string(frame["error"]) != "null" {
			return nil, fmt.Errorf("handler error: %s", frame["error"])
		}
		return frame["result"], nil
	case <-ctx.Done():
		return nil, ctx.Err()
	}
}

func TestDelayedRegisteredNodeReceivesHubSignedLatestConfig(t *testing.T) {
	h, _ := nodeCallHub(t)
	ctx, cancel := context.WithTimeout(t.Context(), 8*time.Second)
	defer cancel()
	r := &registeredNodeRPC{registered: map[string]bool{}, replies: map[string]chan map[string]json.RawMessage{}, notReady: make(chan struct{}, 8)}
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) { r.serve(ctx, w, req) }))
	defer server.Close()
	client := iii.New("ws"+strings.TrimPrefix(server.URL, "http"), iii.WithName("mesh-node"), iii.WithNamespace("default"))
	defer client.Close()
	if err := client.Connect(ctx); err != nil {
		t.Fatal(err)
	}
	h.nodeClient = r
	h.resolveArtifact = func(ctx context.Context, worker, version, target string) (artifact registry.Artifact, err error) {
		return registry.Artifact{Version: version, URL: "https://artifacts.example/" + worker, SHA256: strings.Repeat("a", 64)}, nil
	}
	hooksCalled := 0
	h.SetHooks(Hooks{ConfigChanged: func(context.Context, Config, Config) { hooksCalled++ }})
	h.startNodeCalls(ctx)
	defer h.Close()
	select {
	case <-r.notReady:
	case <-ctx.Done():
		t.Fatal("did not attempt before handler readiness")
	}
	cfg := h.Config()
	cfg.Nodes["laptop"] = NodeConfig{Workers: []string{"ide@0.12"}}
	returned := make(chan error, 1)
	go func() { returned <- h.ApplyConfig(ctx, cfg) }()
	select {
	case err := <-returned:
		if err != nil {
			t.Fatal(err)
		}
	case <-time.After(300 * time.Millisecond):
		t.Fatal("reload held lifecycle lock over unavailable node")
	}
	w := &signedWorkers{applied: make(chan node.WorkersRequest, 8)}
	n, err := node.New(node.Options{Mesh: signedNetwork{}, HubIdentity: h.Coordinator.Identity().Public().(ed25519.PublicKey), Version: h.Version, Operator: "alice", Workers: w})
	if err != nil {
		t.Fatal(err)
	}
	if err = n.Register(client); err != nil {
		t.Fatal(err)
	}
	select {
	case got := <-w.applied:
		if got.Namespace != "project" || len(got.Workers) != 1 || got.Workers[0].Name != "ide" || got.Workers[0].Version != "0.12" {
			t.Fatalf("apply %+v", got)
		}
	case <-ctx.Done():
		t.Fatal("node stranded after delayed registration")
	}
	if hooksCalled != 1 {
		t.Fatalf("config hook calls %d", hooksCalled)
	}
	details, err := h.GetNode(ctx, NodeRequest{Name: "laptop"})
	if err != nil || details.StatusError != "" || details.Version != h.Version || details.Path != "relay hub" || len(details.Workers) != 1 {
		t.Fatalf("details %+v error %v", details, err)
	}
	cancel()
	select {
	case <-h.nodeReconciler.done:
	case <-time.After(time.Second):
		t.Fatal("reconciliation did not stop")
	}
}

type signedDrives struct {
	controller node.DriveController
	mu         sync.Mutex
	applied    []node.DrivesRequest
}

func (d *signedDrives) Apply(ctx context.Context, request node.DrivesRequest) ([]node.DriveStatus, error) {
	d.mu.Lock()
	defer d.mu.Unlock()
	d.applied = append(d.applied, request)
	if d.controller != nil {
		return d.controller.Apply(ctx, request)
	}
	return []node.DriveStatus{}, nil
}
func (d *signedDrives) Status() []node.DriveStatus {
	if d.controller != nil {
		return d.controller.Status()
	}
	return []node.DriveStatus{}
}

func TestResolutionFailuresReachSignedNodeAndDoNotBlockOtherDesiredState(t *testing.T) {
	h, _ := nodeCallHub(t)
	ctx, cancel := context.WithTimeout(t.Context(), 8*time.Second)
	defer cancel()
	r := &registeredNodeRPC{registered: map[string]bool{}, replies: map[string]chan map[string]json.RawMessage{}, notReady: make(chan struct{}, 8)}
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) { r.serve(ctx, w, req) }))
	defer server.Close()
	client := iii.New("ws"+strings.TrimPrefix(server.URL, "http"), iii.WithName("mesh-node"), iii.WithNamespace("default"))
	defer client.Close()
	if err := client.Connect(ctx); err != nil {
		t.Fatal(err)
	}
	driveHost := newHubDriveHost(t)
	workers, drives := &signedWorkers{}, &signedDrives{controller: realDriveController(t, driveHost)}
	n, err := node.New(node.Options{Mesh: signedNetwork{}, HubIdentity: h.Coordinator.Identity().Public().(ed25519.PublicKey), Version: h.Version, Operator: "alice", Workers: workers, Drives: drives})
	if err != nil {
		t.Fatal(err)
	}
	if err = n.Register(client); err != nil {
		t.Fatal(err)
	}
	ticker := time.NewTicker(time.Millisecond)
	defer ticker.Stop()
	for {
		r.mu.Lock()
		ready := len(r.registered) == 4 && r.namespace == "default"
		r.mu.Unlock()
		if ready {
			break
		}
		select {
		case <-ticker.C:
		case <-ctx.Done():
			t.Fatal("handlers not ready")
		}
	}
	h.nodeClient = r
	var resolveFailure error
	h.resolveArtifact = func(_ context.Context, worker, version, target string) (registry.Artifact, error) {
		if worker == "unsupported" && resolveFailure != nil {
			return registry.Artifact{}, resolveFailure
		}
		return registry.Artifact{Version: "1.0.0", URL: "https://artifacts.example/" + worker, SHA256: strings.Repeat("a", 64)}, nil
	}
	for _, failure := range []error{registry.ErrNotBinary, registry.ErrNoTarget} {
		t.Run(failure.Error(), func(t *testing.T) {
			resolveFailure = nil
			h.config.Nodes["laptop"] = NodeConfig{Workers: []string{"old"}}
			if err = h.OnNodeOnline(ctx, "laptop"); err != nil {
				t.Fatal(err)
			}
			resolveFailure = failure
			h.config.Nodes["laptop"] = NodeConfig{Workers: []string{"unsupported", "valid"}}
			h.config.Drives = map[string]DriveConfig{"work": {Server: "laptop", Nodes: []string{"laptop"}}}
			h.applyCoordinator(h.config)
			if err = h.OnNodeOnline(ctx, "laptop"); !errors.Is(err, failure) {
				t.Fatalf("aggregate error %v, want %v", err, failure)
			}
			workers.mu.Lock()
			current := append([]node.Worker{}, workers.workers...)
			started, stopped := append([]string{}, workers.started...), append([]string{}, workers.stopped...)
			workers.mu.Unlock()
			if len(current) != 1 || current[0].Name != "valid" || !slices.Contains(started, "valid") || !slices.Contains(stopped, "old") || slices.Contains(started, "unsupported") {
				t.Fatalf("runnable=%+v started=%v stopped=%v", current, started, stopped)
			}
			drives.mu.Lock()
			lastDrive := drives.applied[len(drives.applied)-1]
			drives.mu.Unlock()
			if len(lastDrive.Mount) != 1 || lastDrive.Mount[0].Name != "work" {
				t.Fatalf("resolution failure blocked drives: %+v", lastDrive)
			}
			status, err := h.fetchNodeStatus(ctx, "laptop")
			if err != nil {
				t.Fatal(err)
			}
			if !slices.ContainsFunc(status.Drives, func(d node.DriveStatus) bool { return d.Name == "work" && d.State == "mounted" }) {
				t.Fatalf("signed manager did not mount: %+v", status.Drives)
			}
			raw, _ := json.Marshal(status)
			var wire struct {
				Workers []struct{ Name, Version, State, Error string }
			}
			json.Unmarshal(raw, &wire)
			states := map[string]struct{ Name, Version, State, Error string }{}
			for _, worker := range wire.Workers {
				states[worker.Name] = worker
			}
			if len(states) != 2 || states["valid"].State != "running" || states["unsupported"].State != "error" || states["unsupported"].Error != failure.Error() {
				t.Fatalf("node status lost failure: %s", raw)
			}

			h.config.Nodes["laptop"] = NodeConfig{Workers: []string{"unsupported"}}
			h.config.Drives = map[string]DriveConfig{"archive": {Server: "laptop", Nodes: []string{"laptop"}}}
			h.applyCoordinator(h.config)
			if err = h.OnNodeOnline(ctx, "laptop"); !errors.Is(err, failure) {
				t.Fatalf("drive-only aggregate error: %v", err)
			}
			workers.mu.Lock()
			running := len(workers.workers)
			workers.mu.Unlock()
			drives.mu.Lock()
			lastDrive = drives.applied[len(drives.applied)-1]
			drives.mu.Unlock()
			if running != 0 || len(lastDrive.Mount) != 1 || lastDrive.Mount[0].Name != "archive" {
				t.Fatalf("all unresolved workers blocked drive-only state: running=%d drives=%+v", running, lastDrive)
			}
		})
	}
	h.config.Nodes["laptop"] = NodeConfig{}
	h.config.Drives = map[string]DriveConfig{"failed": {Server: "laptop", Nodes: []string{"laptop"}}}
	h.applyCoordinator(h.config)
	driveHost.fail = "mount"
	if err = h.OnNodeOnline(ctx, "laptop"); err == nil || !strings.Contains(err.Error(), "injected mount failure") {
		t.Fatalf("drive refusal missing from hub errors: %v", err)
	}
	status, err := h.fetchNodeStatus(ctx, "laptop")
	if err != nil {
		t.Fatal(err)
	}
	if !slices.ContainsFunc(status.Drives, func(d node.DriveStatus) bool { return d.Name == "failed" && d.State == "error" && d.Error != "" }) {
		t.Fatalf("drive failure missing from status: %+v", status.Drives)
	}
	driveHost.fail = ""
	h.config.Drives = nil
	h.applyCoordinator(h.config)
	if err = h.OnNodeOnline(ctx, "laptop"); err != nil {
		t.Fatal(err)
	}
	status, err = h.fetchNodeStatus(ctx, "laptop")
	if err != nil || len(status.Drives) != 0 {
		t.Fatalf("empty signed replacement left resources: %+v %v", status.Drives, err)
	}

}

func TestStalledRegistryDoesNotBlockSignedDesiredState(t *testing.T) {
	for _, stage := range []string{"headers", "body"} {
		t.Run(stage, func(t *testing.T) {
			t.Parallel()
			h, _ := nodeCallHub(t)
			ctx, cancel := context.WithCancel(t.Context())
			defer cancel()
			r := &registeredNodeRPC{registered: map[string]bool{}, replies: map[string]chan map[string]json.RawMessage{}, notReady: make(chan struct{}, 8)}
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) { r.serve(ctx, w, req) }))
			defer server.Close()
			client := iii.New("ws"+strings.TrimPrefix(server.URL, "http"), iii.WithName("mesh-node"), iii.WithNamespace("default"))
			defer client.Close()
			if err := client.Connect(ctx); err != nil {
				t.Fatal(err)
			}
			workers := &signedWorkers{}
			drives := &signedDrives{controller: realDriveController(t, newHubDriveHost(t))}
			n, err := node.New(node.Options{Mesh: signedNetwork{}, HubIdentity: h.Coordinator.Identity().Public().(ed25519.PublicKey), Version: h.Version, Operator: "alice", Workers: workers, Drives: drives})
			if err != nil {
				t.Fatal(err)
			}
			if err := n.Register(client); err != nil {
				t.Fatal(err)
			}
			deadline := time.After(3 * time.Second)
			ticker := time.NewTicker(time.Millisecond)
			defer ticker.Stop()
			for {
				r.mu.Lock()
				ready := len(r.registered) == 4 && r.namespace == "default"
				r.mu.Unlock()
				if ready {
					break
				}
				select {
				case <-ticker.C:
				case <-deadline:
					t.Fatal("handlers not ready")
				}
			}
			registryServer := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
				var request struct{ Worker, Target string }
				if err := json.NewDecoder(req.Body).Decode(&request); err != nil {
					t.Error(err)
					return
				}
				if request.Worker == "stalled" {
					if stage == "body" {
						fmt.Fprint(w, `{"graph":[`)
						w.(http.Flusher).Flush()
					}
					<-req.Context().Done()
					return
				}
				fmt.Fprintf(w, `{"graph":[{"name":%q,"version":"1.0.0","type":"binary","binaries":{%q:{"url":"https://artifacts.example/worker","sha256":%q}}}]}`, request.Worker, request.Target, strings.Repeat("a", 64))
			}))
			t.Cleanup(registryServer.Close)
			t.Cleanup(cancel)
			h.nodeClient = r
			h.resolveArtifact = func(ctx context.Context, worker, version, target string) (registry.Artifact, error) {
				return registry.Resolve(ctx, nil, registryServer.URL, worker, version, target)
			}
			h.config.Nodes["laptop"] = NodeConfig{Workers: []string{"old"}}
			if err := h.OnNodeOnline(ctx, "laptop"); err != nil {
				t.Fatal(err)
			}
			h.config.Nodes["laptop"] = NodeConfig{Workers: []string{"stalled", "valid"}}
			h.config.Drives = map[string]DriveConfig{"work": {Server: "laptop", Nodes: []string{"laptop"}}}
			h.applyCoordinator(h.config)
			done := make(chan error, 1)
			go func() { done <- h.OnNodeOnline(ctx, "laptop") }()
			select {
			case err := <-done:
				if !errors.Is(err, context.DeadlineExceeded) {
					t.Fatalf("aggregate error = %v, want registry deadline exceeded", err)
				}
			case <-time.After(12 * time.Second):
				cancel()
				<-done
				t.Fatal("stalled first lookup blocked complete worker and drive apply")
			}
			if ctx.Err() != nil {
				t.Fatalf("outer reconciliation context canceled: %v", ctx.Err())
			}
			workers.mu.Lock()
			current := append([]node.Worker{}, workers.workers...)
			stopped := append([]string{}, workers.stopped...)
			workers.mu.Unlock()
			if len(current) != 1 || current[0].Name != "valid" || !slices.Contains(stopped, "old") {
				t.Fatalf("runnable=%+v stopped=%v", current, stopped)
			}
			status, err := h.fetchNodeStatus(ctx, "laptop")
			if err != nil {
				t.Fatal(err)
			}
			if len(status.Workers) != 2 || !slices.ContainsFunc(status.Workers, func(w node.WorkerStatus) bool {
				return w.Name == "stalled" && w.State == "error" && strings.Contains(w.Error, "context deadline exceeded")
			}) {
				t.Fatalf("signed node lost per-worker timeout: %+v", status.Workers)
			}
			if !slices.ContainsFunc(status.Drives, func(d node.DriveStatus) bool { return d.Name == "work" && d.State == "mounted" }) {
				t.Fatalf("stalled lookup blocked drive mount: %+v", status.Drives)
			}
			h.config.Nodes["laptop"] = NodeConfig{}
			h.config.Drives = nil
			h.applyCoordinator(h.config)
			if err := h.OnNodeOnline(ctx, "laptop"); err != nil {
				t.Fatalf("subsequent reconciliation failed: %v", err)
			}
			status, err = h.fetchNodeStatus(ctx, "laptop")
			if err != nil || len(status.Workers) != 0 || len(status.Drives) != 0 {
				t.Fatalf("subsequent empty desired state not applied: %+v, %v", status, err)
			}
		})
	}
}
