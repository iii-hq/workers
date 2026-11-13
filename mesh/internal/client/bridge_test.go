package client

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"maps"
	"net/http"
	"net/http/httptest"
	"reflect"
	"slices"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/coder/websocket"
	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/node"
)

type fakeLocal struct {
	mu      sync.Mutex
	calls   []iii.CallRequest
	fn      func(iii.CallRequest) (json.RawMessage, error)
	ctxFn   func(context.Context, iii.CallRequest) (json.RawMessage, error) // replaces fn where a test needs the call's context
	mirrors map[string]iii.Handler                                          // registrations; removals cannot be seen: Unregister of a handle the SDK did not make sends nothing
	opts    map[string]iii.RegisterFunctionOptions
	added   int
}

func (f *fakeLocal) RegisterFunction(id string, h iii.Handler, opts ...iii.RegisterFunctionOptions) (*iii.FunctionRef, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	if f.mirrors == nil {
		f.mirrors, f.opts = map[string]iii.Handler{}, map[string]iii.RegisterFunctionOptions{}
	}
	f.mirrors[id], f.opts[id] = h, opts[0]
	f.added++
	return &iii.FunctionRef{ID: id}, nil
}

func (f *fakeLocal) Call(ctx context.Context, r iii.CallRequest) (json.RawMessage, error) {
	f.mu.Lock()
	f.calls = append(f.calls, r)
	f.mu.Unlock()
	if f.ctxFn != nil {
		return f.ctxFn(ctx, r)
	}
	return f.fn(r)
}
func (f *fakeLocal) called(id string) []iii.CallRequest {
	f.mu.Lock()
	defer f.mu.Unlock()
	var out []iii.CallRequest
	for _, c := range f.calls {
		if c.FunctionID == id {
			out = append(out, c)
		}
	}
	return out
}

func row(id, namespace, worker string) map[string]any {
	return map[string]any{"function_id": id, "namespace": namespace, "worker_name": worker, "description": "about " + id, "metadata": map[string]any{"owner": worker}}
}

// engine fakes engine::functions::list over *rows and answers info with a schema per id.
func engine(rows *[]map[string]any) *fakeLocal {
	return &fakeLocal{fn: func(r iii.CallRequest) (json.RawMessage, error) {
		switch r.FunctionID {
		case "engine::functions::list":
			return json.Marshal(map[string]any{"functions": *rows})
		case "engine::functions::info":
			var in struct {
				IDs []string `json:"function_ids"`
			}
			if err := json.Unmarshal(r.Payload, &in); err != nil {
				return nil, err
			}
			out := []map[string]any{}
			for _, id := range in.IDs {
				out = append(out, map[string]any{"function_id": id, "request_schema": map[string]string{"title": id}})
			}
			return json.Marshal(map[string]any{"functions": out})
		}
		return nil, fmt.Errorf("unexpected call %s", r.FunctionID)
	}}
}

// newBridge connects the hub side to a closed port: registrations are kept
// offline, which is all these tests inspect.
func newBridge(t *testing.T, local localEngine) *Bridge {
	t.Setenv("HOME", t.TempDir()) // the SDK writes an iii ps marker per client
	b := NewBridge(local, "local")
	t.Cleanup(b.Close)
	if err := b.SetRuntime(context.Background(), node.Runtime{Node: "laptop", Namespace: "hub", URL: "ws://127.0.0.1:1"}); err != nil {
		t.Fatal(err)
	}
	return b
}

func registered(b *Bridge) []string {
	b.mu.Lock()
	defer b.mu.Unlock()
	return slices.Sorted(maps.Keys(b.refs))
}

func TestEmptyShareMakesNoListCallAndRegistersNothing(t *testing.T) {
	rows := []map[string]any{row("probe::echo", "local", "probe")}
	local := engine(&rows)
	b := newBridge(t, local)
	if err := b.Sync(context.Background(), []string{"probe"}); err != nil {
		t.Fatal(err)
	}
	if got := registered(b); !slices.Equal(got, []string{"probe::echo"}) {
		t.Fatalf("registered %v", got)
	}
	before := len(local.called("engine::functions::list"))
	if err := b.Sync(context.Background(), nil); err != nil {
		t.Fatal(err)
	}
	if len(local.called("engine::functions::list")) != before {
		t.Fatal("empty share listed functions; the engine treats an empty filter as all")
	}
	if got := registered(b); len(got) != 0 || len(b.Shared("laptop")) != 0 || len(b.Status()) != 0 {
		t.Fatalf("empty share still publishes %v", got)
	}
}

func TestSyncFiltersDeniesAndDiffs(t *testing.T) {
	rows := []map[string]any{
		row("probe::echo", "local", "probe"), row("probe::fail", "local", "probe"),
		row("probe::elsewhere", "other", "probe"), // another namespace
		row("engine::x", "local", "probe"), row("mesh::status", "local", "probe"), row("mesh-node::status", "local", "probe"),
		row("secrets::resolve", "local", "probe"), row("configuration::get", "local", "probe"), row("rbac-proxy::x", "local", "probe"),
		row("approval::resolve", "local", "probe"), // would let another machine approve this one's agent calls
		row("bare", "local", "probe"),              // no "::"
		row("shell::run", "local", "ide"),          // not shared, as from an engine ignoring the filter
	}
	local := engine(&rows)
	b := newBridge(t, local)
	if err := b.Sync(context.Background(), []string{"probe"}); err != nil {
		t.Fatal(err)
	}
	if got := registered(b); !slices.Equal(got, []string{"probe::echo", "probe::fail"}) {
		t.Fatalf("registered %v", got)
	}
	list := local.called("engine::functions::list")
	if len(list) != 1 || string(list[0].Payload) != `{"workers":["probe"]}` {
		t.Fatalf("list calls %+v", list)
	}
	f := b.functions["probe::echo"]
	if f.Opts.Description != "about probe::echo" || string(f.Opts.Metadata) != `{"owner":"probe"}` || string(f.Opts.RequestFormat) != `{"title":"probe::echo"}` || f.Opts.Observable != nil {
		// Observable false would be held back by the SDK on hubs without observe.v2.
		t.Fatalf("copied options %+v", f.Opts)
	}

	rows = []map[string]any{row("probe::echo", "local", "probe"), row("probe::new", "local", "probe")}
	if err := b.Sync(context.Background(), []string{"probe"}); err != nil {
		t.Fatal(err)
	}
	if got := registered(b); !slices.Equal(got, []string{"probe::echo", "probe::new"}) {
		t.Fatalf("after diff registered %v", got)
	}
	info := local.called("engine::functions::info")
	if len(info) != 2 || string(info[1].Payload) != `{"function_ids":["probe::new"],"namespace":"local"}` {
		t.Fatalf("info is fetched for new ids only: %+v", info)
	}
	shared := b.Shared("laptop")
	if len(shared) != 1 || shared[0].State != "pending" || !slices.Equal(shared[0].Functions, []string{"laptop::probe::echo", "laptop::probe::new"}) {
		t.Fatalf("shared %+v", shared)
	}
}

func TestInfoChunksOf32InLocalNamespace(t *testing.T) {
	rows := []map[string]any{}
	for i := range 40 {
		rows = append(rows, row(fmt.Sprintf("probe::f%02d", i), "local", "probe"))
	}
	local := engine(&rows)
	b := newBridge(t, local)
	if err := b.Sync(context.Background(), []string{"probe"}); err != nil {
		t.Fatal(err)
	}
	info := local.called("engine::functions::info")
	sizes := []int{}
	for _, c := range info {
		var in struct {
			IDs       []string `json:"function_ids"`
			Namespace string   `json:"namespace"`
		}
		if err := json.Unmarshal(c.Payload, &in); err != nil || in.Namespace != "local" {
			t.Fatalf("info payload %s", c.Payload)
		}
		sizes = append(sizes, len(in.IDs))
	}
	if !slices.Equal(sizes, []int{32, 8}) || len(registered(b)) != 40 {
		t.Fatalf("info batches %v, registered %d", sizes, len(registered(b)))
	}
}

func TestSetRuntimeIsIdempotentAndEmptyURLSuspends(t *testing.T) {
	rows := []map[string]any{row("probe::echo", "local", "probe")}
	b := newBridge(t, engine(&rows))
	if err := b.Sync(context.Background(), []string{"probe"}); err != nil {
		t.Fatal(err)
	}
	remote, ref := b.remote, b.refs["probe::echo"]
	rt := node.Runtime{Node: "laptop", Namespace: "hub", URL: "ws://127.0.0.1:1"}
	if err := b.SetRuntime(context.Background(), rt); err != nil {
		t.Fatal(err)
	}
	if b.remote != remote || b.refs["probe::echo"] != ref {
		t.Fatal("an unchanged runtime reconnected; the hub apply would loop")
	}
	rt.Namespace = "hub2"
	if err := b.SetRuntime(context.Background(), rt); err != nil {
		t.Fatal(err)
	}
	if b.remote == remote || b.refs["probe::echo"] == nil || b.Namespace() != "hub2" {
		t.Fatal("a changed runtime keeps the old connection")
	}
	if err := b.SetRuntime(context.Background(), node.Runtime{Node: "laptop", Namespace: "hub2"}); err != nil {
		t.Fatal(err)
	}
	if b.remote != nil || len(registered(b)) != 0 {
		t.Fatal("an empty URL does not suspend")
	}
	if s := b.Status(); len(s) != 1 || s[0].Name != "probe" || s[0].State != "pending" || !slices.Equal(s[0].Functions, []string{"probe::echo"}) {
		t.Fatalf("suspended status %+v", s)
	}
}

// A stale twin holding the worker name gets the bridge rejected for good; the
// hub's applies repeat the same runtime, so only Sync or the retry can reconnect it.
func TestSyncReconnectsAfterFatalRejection(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	var conns atomic.Int32
	hub := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer conn.CloseNow()
		if conns.Add(1) == 1 {
			frame, _ := iii.MarshalMessage(&iii.RegistrationRejectedMessage{Code: "WORKER_NAMESPACE_CONFLICT", Namespace: "hub", WorkerName: "rbac-proxy:laptop::mesh-bridge"})
			_ = conn.Write(r.Context(), websocket.MessageText, frame)
		}
		for {
			if _, _, err := conn.Read(r.Context()); err != nil {
				return
			}
		}
	}))
	defer hub.Close()
	rows := []map[string]any{row("probe::echo", "local", "probe")}
	b := NewBridge(engine(&rows), "local")
	defer b.Close()
	rt := node.Runtime{Node: "laptop", Namespace: "hub", URL: "ws" + strings.TrimPrefix(hub.URL, "http")}
	if err := b.SetRuntime(context.Background(), rt); err != nil {
		t.Fatal(err)
	}
	rejected := b.remote
	waitUntil(t, "fatal rejection", func() bool { return rejected.FatalError() != nil })
	if err := b.SetRuntime(context.Background(), rt); err != nil || b.remote != rejected {
		t.Fatalf("an unchanged runtime reconnected: %v", err)
	}
	if err := b.Sync(context.Background(), []string{"probe"}); err != nil {
		t.Fatal(err)
	}
	if b.remote == rejected {
		t.Fatal("Sync kept the rejected client")
	}
	waitUntil(t, "shared after reconnect", func() bool {
		s := b.Shared("laptop")
		return len(s) == 1 && s[0].State == "shared"
	})
}

// A connection the hub rejected is replaced by itself, with no Sync, waiting
// longer each time until the hub accepts one.
func TestBridgeRetriesARejectedConnection(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	defer func(d time.Duration) { bridgeRetry = d }(bridgeRetry)
	bridgeRetry = 50 * time.Millisecond
	var mu sync.Mutex
	var opened []time.Time
	hub := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer conn.CloseNow()
		mu.Lock()
		opened = append(opened, time.Now())
		n := len(opened)
		mu.Unlock()
		var frame []byte
		if n <= 2 {
			frame, _ = iii.MarshalMessage(&iii.RegistrationRejectedMessage{Code: "WORKER_NAMESPACE_CONFLICT", Namespace: "hub", WorkerName: "rbac-proxy:laptop::mesh-bridge"})
		} else {
			frame, _ = json.Marshal(map[string]any{"type": "workerregistered", "worker_id": "bridge"})
		}
		_ = conn.Write(r.Context(), websocket.MessageText, frame)
		for {
			if _, _, err := conn.Read(r.Context()); err != nil {
				return
			}
		}
	}))
	defer hub.Close()
	rows := []map[string]any{row("probe::echo", "local", "probe")}
	local := engine(&rows)
	b := NewBridge(local, "local")
	var changes atomic.Int32
	b.Changed = func() { changes.Add(1) }
	defer b.Close()
	if err := b.Sync(context.Background(), []string{"probe"}); err != nil {
		t.Fatal(err)
	}
	if err := b.SetRuntime(context.Background(), node.Runtime{Node: "laptop", Namespace: "hub", URL: "ws" + strings.TrimPrefix(hub.URL, "http")}); err != nil {
		t.Fatal(err)
	}
	// The reconnects keep the mirrors: clearing them would fire a local function change, and a Sync, for each.
	b.SetCatalog(node.Catalog{Rev: "r1", Functions: []node.CatalogEntry{{ID: "shell::exec", Owner: "hub"}}})
	waitUntil(t, "shared after two rejections", func() bool {
		s := b.Shared("laptop")
		return len(s) == 1 && s[0].State == "shared"
	})
	waitUntil(t, "the backoff ends once the hub accepts", func() bool {
		b.mu.Lock()
		defer b.mu.Unlock()
		return b.backoff == 0
	})
	mu.Lock()
	defer mu.Unlock()
	if len(opened) != 3 || opened[1].Sub(opened[0]) < bridgeRetry || opened[2].Sub(opened[1]) < 2*bridgeRetry || changes.Load() == 0 {
		t.Fatalf("connections at %v, %d changes", opened, changes.Load())
	}
	if b.CatalogRev() != "r1" || local.added != 1 || len(b.Remote()) != 1 {
		t.Fatalf("mirrors after reconnects: rev %q, %d registrations", b.CatalogRev(), local.added)
	}
}

func waitUntil(t *testing.T, what string, ok func() bool) {
	t.Helper()
	for deadline := time.Now().Add(5 * time.Second); !ok(); time.Sleep(20 * time.Millisecond) {
		if time.Now().After(deadline) {
			t.Fatal("timed out waiting for " + what)
		}
	}
}

func TestStateForMissingWorker(t *testing.T) {
	rows := []map[string]any{}
	b := newBridge(t, engine(&rows))
	if err := b.Sync(context.Background(), []string{"probe"}); err != nil {
		t.Fatal(err)
	}
	want := []node.WorkerStatus{{Name: "probe", State: "not running", Functions: []string{}}}
	if got := b.Status(); !reflect.DeepEqual(got, want) || got[0].Functions == nil {
		t.Fatalf("status %+v", got)
	}
}

func TestForwardKeepsPayloadAndErrorCode(t *testing.T) {
	payload := json.RawMessage(`{"b": [1, 2.50],"a":"x"}`)
	local := &fakeLocal{fn: func(r iii.CallRequest) (json.RawMessage, error) {
		if r.FunctionID == "probe::fail" {
			return nil, &iii.InvocationError{Code: "PROBE_FAILED", Message: "no"}
		}
		return r.Payload, nil
	}}
	b := NewBridge(local, "local")
	out, err := b.forward("probe::echo")(context.Background(), payload)
	if err != nil || !bytes.Equal(out.(json.RawMessage), payload) {
		t.Fatalf("echo %s %v", out, err)
	}
	c := local.called("probe::echo")[0]
	if c.Namespace != "local" || c.Timeout != 10*time.Minute || !bytes.Equal(c.Payload, payload) {
		t.Fatalf("local call %+v", c)
	}
	_, err = b.forward("probe::fail")(context.Background(), payload)
	var ie *iii.InvocationError
	if !errors.As(err, &ie) || ie.Code != "PROBE_FAILED" {
		t.Fatalf("error code lost: %v", err)
	}
}

func TestForwardRefusesChannelRefsBothWays(t *testing.T) {
	ref := `{"data":{"reader":{"channel_id":"c","access_key":"k","direction":"read"}}}`
	local := &fakeLocal{fn: func(r iii.CallRequest) (json.RawMessage, error) { return json.RawMessage(ref), nil }}
	b := NewBridge(local, "local")
	var ie *iii.InvocationError
	if _, err := b.forward("probe::echo")(context.Background(), json.RawMessage(ref)); !errors.As(err, &ie) || ie.Code != "channels_unsupported" || len(local.called("probe::echo")) != 0 {
		t.Fatalf("request with a channel ref: %v", err)
	}
	if _, err := b.forward("probe::echo")(context.Background(), json.RawMessage(`{}`)); !errors.As(err, &ie) || ie.Code != "channels_unsupported" {
		t.Fatalf("result with a channel ref: %v", err)
	}
}

func TestCatalogMirrors(t *testing.T) {
	rows := []map[string]any{}
	local := engine(&rows)
	b := newBridge(t, local)
	catalog := node.Catalog{Rev: "r1", Functions: []node.CatalogEntry{
		{ID: "shell::exec", Owner: "hub", Description: "run", RequestFormat: json.RawMessage(`{"type":"object"}`)},
		{ID: "desk::coder::read-file", Owner: "desk"},
		{ID: "laptop::probe::echo", Owner: "laptop"}, // its own
	}}
	b.SetCatalog(catalog)
	if got := slices.Sorted(maps.Keys(local.mirrors)); !slices.Equal(got, []string{"hub::shell::exec", "peer::desk::coder::read-file"}) {
		t.Fatalf("mirrors %v", got)
	}
	if o := local.opts["hub::shell::exec"]; o.Description != "run" || string(o.RequestFormat) != `{"type":"object"}` || o.Metadata != nil {
		t.Fatalf("mirror options %+v", o)
	}
	if got := b.Remote(); b.CatalogRev() != "r1" || !reflect.DeepEqual(got, []RemoteStatus{{"hub", []string{"hub::shell::exec"}, ""}, {"desk", []string{"peer::desk::coder::read-file"}, "hub"}}) {
		t.Fatalf("remote %+v rev %q", got, b.CatalogRev())
	}
	var ie *iii.InvocationError
	if _, err := local.mirrors["hub::shell::exec"](context.Background(), json.RawMessage(`{}`)); !errors.As(err, &ie) || ie.Code != "mesh_offline" {
		t.Fatalf("offline call: %v", err)
	}
	// A mirror is never published back, also when the mesh worker runs under a shared name.
	rows = []map[string]any{row("hub::shell::exec", "local", "mesh-renamed"), row("probe::echo", "local", "mesh-renamed")}
	if err := b.Sync(context.Background(), []string{"mesh-renamed"}); err != nil {
		t.Fatal(err)
	}
	if got := registered(b); !slices.Equal(got, []string{"probe::echo"}) {
		t.Fatalf("published %v", got)
	}
	b.SetCatalog(catalog)
	rt := node.Runtime{Node: "laptop", Namespace: "hub", URL: "ws://127.0.0.1:1"}
	if err := b.SetRuntime(context.Background(), rt); err != nil || local.added != 2 || b.CatalogRev() != "r1" {
		t.Fatalf("the same catalog and runtime registered again: %d registrations, rev %q, %v", local.added, b.CatalogRev(), err)
	}
	b.SetCatalog(node.Catalog{Rev: "r2", Functions: catalog.Functions[1:]})
	if b.mirrors["hub::shell::exec"] != nil || len(b.Remote()) != 1 {
		t.Fatalf("a dropped entry stays mirrored: %v", b.Remote())
	}
	rt.Node = "work" // a rename
	if err := b.SetRuntime(context.Background(), rt); err != nil || b.CatalogRev() != "" || len(b.Remote()) != 0 || len(b.mirrors) != 0 {
		t.Fatalf("a changed runtime keeps the catalog: rev %q %v", b.CatalogRev(), b.Remote())
	}
}

// meshHub accepts the bridge and answers mesh::call with what it received, or drops the connection instead. It also hands over each mesh::node::changed.
func meshHub(t *testing.T, drop bool) (string, chan map[string]json.RawMessage) {
	calls := make(chan map[string]json.RawMessage, 16)
	hub := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer conn.CloseNow()
		hello, _ := json.Marshal(map[string]any{"type": "workerregistered", "worker_id": "bridge"})
		_ = conn.Write(r.Context(), websocket.MessageText, hello)
		for {
			_, raw, err := conn.Read(r.Context())
			if err != nil {
				return
			}
			var f map[string]json.RawMessage
			if json.Unmarshal(raw, &f) != nil || string(f["type"]) != `"invokefunction"` {
				continue
			}
			if id := string(f["function_id"]); id == `"mesh::node::changed"` {
				calls <- f
				continue
			} else if id != `"mesh::call"` {
				continue
			}
			calls <- f
			if drop {
				return
			}
			reply, _ := json.Marshal(map[string]any{"type": "invocationresult", "invocation_id": f["invocation_id"], "function_id": f["function_id"], "result": map[string]json.RawMessage{"got": f["data"]}})
			_ = conn.Write(r.Context(), websocket.MessageText, reply)
		}
	}))
	t.Cleanup(hub.Close)
	return "ws" + strings.TrimPrefix(hub.URL, "http"), calls
}

func TestMirrorCallsThroughTheHub(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	for _, drop := range []bool{false, true} {
		url, calls := meshHub(t, drop)
		local := engine(&[]map[string]any{})
		b := NewBridge(local, "local")
		defer b.Close()
		if err := b.SetRuntime(context.Background(), node.Runtime{Node: "laptop", Namespace: "hub", URL: url}); err != nil {
			t.Fatal(err)
		}
		b.SetCatalog(node.Catalog{Rev: "r1", Functions: []node.CatalogEntry{{ID: "desk::coder::read-file", Owner: "desk"}}})
		waitUntil(t, "connected", func() bool {
			b.mu.Lock()
			defer b.mu.Unlock()
			return b.online != nil && b.online.Err() == nil
		})
		start := time.Now()
		ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second) // the SDK alone would wait 620 s
		defer cancel()
		out, err := local.mirrors["peer::desk::coder::read-file"](ctx, json.RawMessage(`{"path":"a"}`))
		f := <-calls
		var data any
		json.Unmarshal(f["data"], &data)
		if string(f["namespace"]) != `"hub"` || !reflect.DeepEqual(data, map[string]any{"owner": "desk", "function_id": "desk::coder::read-file", "payload": map[string]any{"path": "a"}}) {
			t.Fatalf("hub got %s", mustMarshal(f))
		}
		var ie *iii.InvocationError
		switch {
		case !drop && (err != nil || string(out.(json.RawMessage)) != `{"got":{"function_id":"desk::coder::read-file","owner":"desk","payload":{"path":"a"}}}`):
			t.Fatalf("result %s: %v", out, err)
		case drop && (!errors.As(err, &ie) || ie.Code != "mesh_offline" || time.Since(start) > 5*time.Second):
			t.Fatalf("a dropped connection left the call: %v after %s", err, time.Since(start))
		}
	}
}

func isOnline(b *Bridge) bool {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.online != nil && b.online.Err() == nil
}

// nextFrame returns the next frame the hub got, or fails after 5 s.
func nextFrame(t *testing.T, frames chan map[string]json.RawMessage) map[string]json.RawMessage {
	t.Helper()
	select {
	case f := <-frames:
		return f
	case <-time.After(5 * time.Second):
		t.Fatal("the hub got nothing")
		return nil
	}
}

// noFrame fails when the hub gets a frame within d.
func noFrame(t *testing.T, frames chan map[string]json.RawMessage, d time.Duration) {
	t.Helper()
	select {
	case f := <-frames:
		t.Fatalf("the hub got %s", mustMarshal(f))
	case <-time.After(d):
	}
}

// Announce sends one void mesh::node::changed in the hub's namespace, and nothing while the hub connection is down.
func TestAnnounce(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	url, frames := meshHub(t, false)
	b := NewBridge(engine(&[]map[string]any{}), "local")
	defer b.Close()
	if b.Announce() {
		t.Fatal("announced with no hub connection")
	}
	if err := b.SetRuntime(context.Background(), node.Runtime{Node: "laptop", Namespace: "hub", URL: "ws://127.0.0.1:1"}); err != nil {
		t.Fatal(err)
	}
	if b.Announce() {
		t.Fatal("announced while the hub connection is not up")
	}
	if err := b.SetRuntime(context.Background(), node.Runtime{Node: "laptop", Namespace: "hub", URL: url}); err != nil {
		t.Fatal(err)
	}
	waitUntil(t, "connected", func() bool { return isOnline(b) })
	if !b.Announce() {
		t.Fatal("not sent on a connection that is up")
	}
	if f := nextFrame(t, frames); string(f["function_id"]) != `"mesh::node::changed"` || string(f["namespace"]) != `"hub"` || f["invocation_id"] != nil {
		t.Fatalf("the hub got %s", mustMarshal(f))
	}
	noFrame(t, frames, 100*time.Millisecond)
}

func mustMarshal(v any) string { b, _ := json.Marshal(v); return string(b) }

// The page reads remote from mesh::status: the hub first, as local mirror ids.
func TestStatusListsRemote(t *testing.T) {
	r := &runner{bridge: newBridge(t, engine(&[]map[string]any{}))}
	r.bridge.SetCatalog(node.Catalog{Rev: "r1", Functions: []node.CatalogEntry{{ID: "alpha::probe::echo", Owner: "alpha"}, {ID: "shell::exec", Owner: "hub"}}})
	if b, _ := json.Marshal(r.status()); !strings.Contains(string(b), `"remote":[{"machine":"hub","functions":["hub::shell::exec"]},{"machine":"alpha","functions":["peer::alpha::probe::echo"],"path":"hub"}]`) {
		t.Fatalf("status %s", b)
	}
}
