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
	"reflect"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/coder/websocket"
	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/meshnet"
	"github.com/iii-hq/workers/mesh/internal/node"
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

type signedWorkers struct{ runtimes chan node.Runtime }

func (w *signedWorkers) SetRuntime(_ context.Context, r node.Runtime) error {
	w.runtimes <- r
	return nil
}
func (w *signedWorkers) SetCatalog(node.Catalog) {}
func (w *signedWorkers) CatalogRev() string      { return "" }
func (w *signedWorkers) Status() []node.WorkerStatus {
	return []node.WorkerStatus{{Name: "shell", State: "shared", Functions: []string{"shell::exec"}}}
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
func (r *registeredNodeRPC) Call(ctx context.Context, req iii.CallRequest) (json.RawMessage, error) {
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
	json.Unmarshal(req.Payload, &payload)
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

func TestDelayedRegisteredNodeReceivesHubSignedNamespace(t *testing.T) {
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
	cfg.TrustedProxies = []string{"127.0.0.1/32"}
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
	w := &signedWorkers{runtimes: make(chan node.Runtime, 8)}
	n, err := node.New(node.Options{Mesh: signedNetwork{}, HubIdentity: h.Coordinator.Identity().Public().(ed25519.PublicKey), Version: h.Version, Workers: w, URL: "ws://unused"})
	if err != nil {
		t.Fatal(err)
	}
	if err = n.Register(client); err != nil {
		t.Fatal(err)
	}
	select {
	case got := <-w.runtimes:
		if got.Namespace != "project" || got.Node != "laptop" {
			t.Fatalf("runtime %+v", got)
		}
	case <-ctx.Done():
		t.Fatal("node stranded after delayed registration")
	}
	if hooksCalled != 1 {
		t.Fatalf("config hook calls %d", hooksCalled)
	}
	details, err := h.GetNode(ctx, NodeRequest{Name: "laptop"})
	if err != nil || details.StatusError != "" || details.Version != h.Version || details.Path != "relay hub" || !reflect.DeepEqual(details.Workers, []ComponentState{{Name: "shell", State: "shared", Functions: []string{"laptop::shell::exec"}}}) {
		t.Fatalf("details %+v error %v", details, err)
	}
	cancel()
	select {
	case <-h.nodeReconciler.done:
	case <-time.After(time.Second):
		t.Fatal("reconciliation did not stop")
	}
}
