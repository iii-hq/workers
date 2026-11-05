package hub

import (
	"context"
	"crypto/rand"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/coordinator"
	"github.com/iii-hq/workers/mesh/internal/meshnet"
	"tailscale.com/tsnet"
)

type eventWire struct {
	t           *testing.T
	ctx         context.Context
	conn        *websocket.Conn
	frames      chan map[string]json.RawMessage
	connections chan *websocket.Conn
	revision    int
}

func newEventWire(t *testing.T, h *Hub) *eventWire {
	t.Helper()
	ctx, cancel := context.WithTimeout(t.Context(), 10*time.Second)
	t.Cleanup(cancel)
	e := &eventWire{t: t, ctx: ctx, frames: make(chan map[string]json.RawMessage, 32)}
	connected := make(chan *websocket.Conn, 1)
	e.connections = connected
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer conn.CloseNow()
		connected <- conn
		greet, _ := json.Marshal(map[string]any{"type": "workerregistered", "worker_id": "event-wire", "features": []string{"observe.v1", "observe.v2", "local-dispatch.v1"}})
		conn.Write(ctx, websocket.MessageText, greet)
		for {
			_, raw, err := conn.Read(ctx)
			if err != nil {
				return
			}
			var frame map[string]json.RawMessage
			if json.Unmarshal(raw, &frame) != nil {
				return
			}
			if string(frame["function_id"]) == `"engine::workers::register"` || string(frame["type"]) == `"reattach"` {
				continue
			}
			if string(frame["type"]) == `"registerfunction"` {
				confirm, _ := json.Marshal(map[string]any{"type": "functionregistered", "id": "mesh::on-node-changed", "namespace": "hub-project", "revision": 0, "local_dispatch": true, "observers": []any{}})
				conn.Write(ctx, websocket.MessageText, confirm)
			}
			if string(frame["type"]) == `"invocationsobserved"` || string(frame["type"]) == `"localinvocationstats"` {
				continue
			}
			select {
			case e.frames <- frame:
			case <-ctx.Done():
				return
			}
		}
	}))
	t.Cleanup(server.Close)
	client := iii.New("ws"+strings.TrimPrefix(server.URL, "http"), iii.WithNamespace("hub-project"), iii.WithReconnectConfig(iii.ReconnectConfig{InitialDelay: 2 * time.Second, MaxDelay: 2 * time.Second, MaxRetries: -1}))
	t.Cleanup(func() { client.Close() })
	if err := client.Connect(ctx); err != nil {
		t.Fatal(err)
	}
	e.conn = <-connected
	if err := h.registerNodeEvents(ctx, client); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { h.Close() })
	f := e.next("registerfunction")
	if string(f["id"]) != `"mesh::on-node-changed"` {
		t.Fatalf("type: %s", f)
	}
	if string(f["private"]) != "true" {
		t.Fatalf("node event is not private: %s", f)
	}
	return e
}
func (e *eventWire) send(frame any) {
	e.t.Helper()
	b, err := json.Marshal(frame)
	if err != nil {
		e.t.Fatal(err)
	}
	if err = e.conn.Write(e.ctx, websocket.MessageText, b); err != nil {
		e.t.Fatal(err)
	}
}
func (e *eventWire) next(kind string) map[string]json.RawMessage {
	e.t.Helper()
	select {
	case f := <-e.frames:
		if string(f["type"]) != fmt.Sprintf("%q", kind) {
			e.t.Fatalf("got %s, want %s", f, kind)
		}
		return f
	case <-e.ctx.Done():
		e.t.Fatal("missing SDK frame", kind, e.ctx.Err())
		return nil
	}
}
func (e *eventWire) bind(id, cfg string) bool {
	e.t.Helper()
	e.revision++
	e.send(map[string]any{"type": "observeradded", "function_id": "mesh::on-node-changed", "namespace": "hub-project", "revision": e.revision, "observer": map[string]any{"id": id, "args": json.RawMessage(cfg), "filters": map[string]any{}, "target": "audit::" + id, "capture": []string{"input", "output", "caller"}, "on": "success", "deliver": "output"}})
	f := e.next("observerack")
	return len(f["error"]) == 0 || string(f["error"]) == "null"
}
func (e *eventWire) ack(f map[string]json.RawMessage) {
	e.send(map[string]any{"type": "invocationresult", "invocation_id": f["invocation_id"], "function_id": f["function_id"], "result": nil})
}
func (e *eventWire) delivery(name, event string) map[string]json.RawMessage {
	e.t.Helper()
	f := e.next("invokefunction")
	var payload map[string]json.RawMessage
	if json.Unmarshal(f["data"], &payload) != nil || len(payload) != 3 || string(payload["name"]) != fmt.Sprintf("%q", name) || string(payload["event"]) != fmt.Sprintf("%q", event) {
		e.t.Fatalf("payload %s", f["data"])
	}
	var at time.Time
	if json.Unmarshal(payload["at"], &at) != nil || at.IsZero() {
		e.t.Fatal("missing timestamp")
	}
	if len(f["activation"]) == 0 {
		e.t.Fatalf("delivery lacks observer activation: %s", f)
	}
	return f
}
func eventVersion(t *testing.T, h *Hub, version string) {
	t.Helper()
	if err := h.Coordinator.SetRuntimeMetadata("laptop", coordinator.RuntimeMetadata{OS: "linux", Arch: "amd64", Version: version}); err != nil {
		t.Fatal(err)
	}
}
func TestBindingRejectsUnknownKeys(t *testing.T) {
	h := &Hub{}
	e := newEventWire(t, h)
	for _, cfg := range []string{`{"unknown":true}`, `{"condition_function_id":"evil::call"}`, `{"node":"laptop","extra":0}`, `{"node":null}`, `{"node":3}`, `[]`, `{"node":""}`} {
		if e.bind("bad", cfg) {
			t.Fatalf("accepted %s", cfg)
		}
	}
	for i, cfg := range []string{`{}`, `null`, `{"node":"laptop"}`} {
		if !e.bind(fmt.Sprint(i), cfg) {
			t.Fatalf("refused %s", cfg)
		}
	}
}
func TestDeliversOnlyForBoundNode(t *testing.T) {
	h, _ := nodeCallHub(t)
	e := newEventWire(t, h)
	if !e.bind("other", `{"node":"desktop"}`) || !e.bind("laptop", `{"node":"laptop"}`) {
		t.Fatal("bind")
	}
	eventVersion(t, h, "0.1.0")
	f := e.delivery("laptop", "outdated")
	if string(f["activation"]) != `{"observer_id":"laptop"}` {
		t.Fatalf("filter leaked: %s", f)
	}
	e.ack(f)
	eventVersion(t, h, "0.1.0")
	eventVersion(t, h, "0.2.0")
	if err := h.Coordinator.Remove("laptop"); err != nil {
		t.Fatal(err)
	}
	f = e.delivery("laptop", "removed")
	if string(f["activation"]) != `{"observer_id":"laptop"}` {
		t.Fatalf("filter leaked: %s", f)
	}
	e.ack(f)
	select {
	case f := <-e.frames:
		t.Fatalf("unexpected delivery %s", f)
	case <-time.After(30 * time.Millisecond):
	}
}
func TestNoReplayForLateBinding(t *testing.T) {
	h, _ := nodeCallHub(t)
	e := newEventWire(t, h)
	eventVersion(t, h, "0.0.1")
	if !e.bind("early", `{}`) {
		t.Fatal("bind")
	}
	eventVersion(t, h, "0.0.2")
	first := e.delivery("laptop", "outdated")
	eventVersion(t, h, "0.0.3")
	if !e.bind("late", `{}`) {
		t.Fatal("bind")
	}
	e.ack(first)
	second := e.delivery("laptop", "outdated")
	if string(second["activation"]) != `{"observer_id":"early"}` {
		t.Fatalf("queued history reached late binding: %s", second)
	}
	e.ack(second)
	select {
	case f := <-e.frames:
		t.Fatalf("replayed %s", f)
	case <-time.After(30 * time.Millisecond):
	}
	if err := h.Coordinator.Remove("laptop"); err != nil {
		t.Fatal(err)
	}
	seen := map[string]bool{}
	for range 2 {
		f := e.delivery("laptop", "removed")
		seen[string(f["activation"])] = true
		e.ack(f)
	}
	if len(seen) != 2 {
		t.Fatal("fresh transition missing", seen)
	}
}

func TestNodeEventsUnregisterReplacementAndClose(t *testing.T) {
	h, _ := nodeCallHub(t)
	e := newEventWire(t, h)
	if !e.bind("binding", `{}`) {
		t.Fatal("bind")
	}
	eventVersion(t, h, "old-1")
	first := e.delivery("laptop", "outdated")
	eventVersion(t, h, "old-2")
	if !e.bind("binding", `{"node":"desktop"}`) {
		t.Fatal("replace")
	}
	e.ack(first)
	select {
	case f := <-e.frames:
		t.Fatalf("replacement received queued history %s", f)
	case <-time.After(30 * time.Millisecond):
	}
	e.revision++
	e.send(map[string]any{"type": "observerremoved", "function_id": "mesh::on-node-changed", "namespace": "hub-project", "revision": e.revision, "id": "binding"})
	deadline := time.Now().Add(time.Second)
	for {
		h.events.mu.Lock()
		n := len(h.events.bindings)
		h.events.mu.Unlock()
		if n == 0 {
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("SDK unregistration retained binding")
		}
		time.Sleep(time.Millisecond)
	}
	eventVersion(t, h, "old-3")
	if !e.bind("active", `{}`) {
		t.Fatal("bind after unregister")
	}
	eventVersion(t, h, "old-4")
	e.delivery("laptop", "outdated")
	closed := make(chan struct{})
	go func() { h.Close(); close(closed) }()
	select {
	case <-closed:
	case <-time.After(time.Second):
		t.Fatal("close waited for remote callback")
	}
	if e.bind("after-close", `{}`) {
		t.Fatal("closed hub accepted binding")
	}
	select {
	case f := <-e.frames:
		t.Fatalf("unregistered delivery %s", f)
	case <-time.After(30 * time.Millisecond):
	}
}

func TestNodeEventsBoundedQueueDoesNotBlockCoordinator(t *testing.T) {
	h, _ := nodeCallHub(t)
	e := newEventWire(t, h)
	if !e.bind("slow", `{}`) {
		t.Fatal("bind")
	}
	eventVersion(t, h, "old-0")
	e.delivery("laptop", "outdated")
	published := make(chan error, 1)
	go func() {
		for i := 1; i <= 300; i++ {
			if err := h.Coordinator.SetRuntimeMetadata("laptop", coordinator.RuntimeMetadata{OS: "linux", Arch: "amd64", Version: fmt.Sprintf("old-%d", i)}); err != nil {
				published <- err
				return
			}
		}
		published <- h.Coordinator.Remove("laptop")
	}()
	select {
	case err := <-published:
		if err != nil {
			t.Fatal(err)
		}
	case <-time.After(time.Second):
		t.Fatal("slow SDK blocked committed transitions")
	}
	if len(h.Coordinator.Nodes()) != 0 {
		t.Fatal("remove blocked")
	}
	if len(h.events.queue) >= 300 {
		t.Fatal("delivery queue grew without bound")
	}
	h.Close()
}

func TestNodeEventsRealPeerTransitionsPreserveRevocation(t *testing.T) {
	c, err := coordinator.New(t.TempDir(), func(string, string) (string, error) { return "", nil })
	if err != nil {
		t.Fatal(err)
	}
	h := &Hub{Coordinator: c, Version: "0.2.0", Sessions: &Sessions{}}
	revoked := make(chan string, 1)
	h.SetHooks(Hooks{RevokeNode: func(name string) { revoked <- name }})
	c.SetGone(h.revoke)
	e := newEventWire(t, h)
	if !e.bind("peer", `{"node":"laptop"}`) {
		t.Fatal("bind")
	}
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
	if _, err = peer.Up(e.ctx); err != nil {
		t.Fatal(err)
	}
	for _, event := range []string{"joined", "online"} {
		e.ack(e.delivery("laptop", event))
	}
	peer.Close()
	e.ack(e.delivery("laptop", "offline"))
	if err = c.Remove("laptop"); err != nil {
		t.Fatal(err)
	}
	e.ack(e.delivery("laptop", "removed"))
	select {
	case name := <-revoked:
		if name != "laptop" {
			t.Fatal(name)
		}
	case <-e.ctx.Done():
		t.Fatal("lost revoke hook")
	}
}

func TestNodeEventsReconnectDropsOldBindingsAndKeepsNamespace(t *testing.T) {
	h, _ := nodeCallHub(t)
	e := newEventWire(t, h)
	if !e.bind("same", `{}`) {
		t.Fatal("bind")
	}
	eventVersion(t, h, "before-disconnect")
	first := e.delivery("laptop", "outdated")
	eventVersion(t, h, "queued-before-disconnect")
	h.events.mu.Lock()
	old := h.events.bindings["same"]
	h.events.mu.Unlock()
	e.conn.CloseNow()
	select {
	case <-old.ctx.Done():
	case <-time.After(time.Second):
		t.Fatal("SDK disconnect did not cancel binding")
	}
	eventVersion(t, h, "while-disconnected")
	select {
	case e.conn = <-e.connections:
	case <-e.ctx.Done():
		t.Fatal("no SDK reconnect")
	}
	f := e.next("registerfunction")
	if string(f["id"]) != `"mesh::on-node-changed"` {
		t.Fatalf("reconnected type %s", f)
	}
	eventVersion(t, h, "before-rebinding")
	if !e.bind("same", `{}`) {
		t.Fatal("rebind")
	}
	eventVersion(t, h, "after-rebinding")
	f = e.next("invokefunction")
	if string(f["activation"]) != `{"observer_id":"same"}` || string(f["data"]) == "" {
		t.Fatalf("reconnected delivery %s", f)
	}
	if string(f["invocation_id"]) == string(first["invocation_id"]) {
		t.Fatal("old invocation replayed")
	}
	e.ack(f)
	select {
	case f := <-e.frames:
		t.Fatalf("disconnected history replayed %s", f)
	case <-time.After(30 * time.Millisecond):
	}
}

func TestNodeEventsBindingCapacityAllowsReplacement(t *testing.T) {
	h := &Hub{}
	e := newEventWire(t, h)
	for i := 0; i < 256; i++ {
		if !e.bind(fmt.Sprint(i), `{}`) {
			t.Fatalf("binding %d refused", i)
		}
	}
	if e.bind("overflow", `{}`) {
		t.Fatal("unbounded bindings")
	}
	if !e.bind("0", `{"node":"laptop"}`) {
		t.Fatal("replacement refused at capacity")
	}
}

func TestNodeEventsStaleUnregisterPreservesReconnectedBinding(t *testing.T) {
	h, _ := nodeCallHub(t)
	e := newEventWire(t, h)
	if !e.bind("same", `{}`) {
		t.Fatal("bind")
	}
	e.revision++
	e.send(map[string]any{"type": "observerremoved", "function_id": "mesh::on-node-changed", "namespace": "hub-project", "revision": e.revision, "id": "same"})
	if !e.bind("same", `{"node":"laptop"}`) {
		t.Fatal("replacement")
	}
	// An old revision cannot remove the new generation.
	e.send(map[string]any{"type": "observerremoved", "function_id": "mesh::on-node-changed", "namespace": "hub-project", "revision": e.revision - 1, "id": "same"})
	eventVersion(t, h, "fresh")
	e.ack(e.delivery("laptop", "outdated"))
}
