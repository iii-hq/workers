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
	mu    sync.Mutex
	calls []iii.CallRequest
	fn    func(iii.CallRequest) (json.RawMessage, error)
	ctxFn func(context.Context, iii.CallRequest) (json.RawMessage, error) // replaces fn where a test needs the call's context
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
	if got := registered(b); len(got) != 0 || len(b.Shared()) != 0 || len(b.Status()) != 0 {
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
	shared := b.Shared()
	if len(shared) != 1 || shared[0].State != "pending" || !slices.Equal(shared[0].Functions, []string{"probe::echo", "probe::new"}) {
		t.Fatalf("shared %+v", shared)
	}
}

func TestValidateShare(t *testing.T) {
	if err := ValidateShare([]string{"probe", "harness::send", "coder::create-file", "ade::watch::r1"}); err != nil {
		t.Fatal(err)
	}
	for _, bad := range []string{"engine::functions::list", "mesh::status", "mesh-node::status", "secrets::resolve", "configuration::get",
		"rbac-proxy::x", "approval::resolve", "harness::*", "*::send", " harness::send",
		"harness::send ", "harness::", "::send", "rbac-proxy:laptop::mesh-bridge", "mesh", "mesh-mirror", "Bad", strings.Repeat("a", 251) + "::send"} {
		if err := ValidateShare([]string{bad}); err == nil {
			t.Fatalf("accepted %q", bad)
		}
	}
	if err := ValidateShare([]string{"mesh::status"}); err == nil || !strings.Contains(err.Error(), "never leave a machine") {
		t.Fatalf("denied prefix error %v", err)
	}
}

func internal(r map[string]any) map[string]any {
	r["metadata"] = map[string]any{"internal": true}
	return r
}

func TestSyncSharesFunctionsByID(t *testing.T) {
	rows := []map[string]any{
		row("probe::echo", "local", "probe"),
		internal(row("probe::grant", "local", "probe")), // a worker entry shares no internal function
		internal(row("harness::send", "local", "harness")),
		internal(row("harness::stop", "local", "harness")),
		row("harness::status", "local", "harness"),
		internal(row("mesh::status", "local", "harness")), // cited, as a list ValidateShare never saw, yet never bridgeable
		row("vault::get", "local", "secrets"),             // cited, but a denied worker's function under any id
	}
	local := engine(&rows)
	b := newBridge(t, local)
	if err := b.Sync(context.Background(), []string{"probe"}); err != nil {
		t.Fatal(err)
	}
	if got := registered(b); !slices.Equal(got, []string{"probe::echo"}) {
		t.Fatalf("worker entry registered %v", got)
	}
	// An id names no worker, so the listing covers every worker and the rows the engine hides.
	if err := b.Sync(context.Background(), []string{"probe", "harness::send", "probe::grant", "mesh::status", "vault::get"}); err != nil {
		t.Fatal(err)
	}
	if got := registered(b); !slices.Equal(got, []string{"harness::send", "probe::echo", "probe::grant"}) {
		t.Fatalf("id entries registered %v", got)
	}
	if list := local.called("engine::functions::list"); string(list[len(list)-1].Payload) != `{"include_internal":true}` {
		t.Fatalf("list payload %s", list[len(list)-1].Payload)
	}
	if f := b.functions["harness::send"]; f.Worker != "harness" || !Internal(f.Opts.Metadata) {
		t.Fatalf("an id entry's copy is no longer internal: %+v", f)
	}
	want := []SharedStatus{
		{Worker: "probe", State: "pending", Functions: []string{"probe::echo"}},
		{Worker: "harness::send", State: "pending", Functions: []string{"harness::send"}},
		{Worker: "probe::grant", State: "pending", Functions: []string{"probe::grant"}},
		{Worker: "mesh::status", State: "not running", Functions: []string{}},
		{Worker: "vault::get", State: "not running", Functions: []string{}},
	}
	if got := b.Shared(); !reflect.DeepEqual(got, want) {
		t.Fatalf("shared %+v", got)
	}
	// A failed read keeps what is known, but an id taken off the list leaves at once.
	local.fn = func(iii.CallRequest) (json.RawMessage, error) { return nil, errors.New("down") }
	if err := b.Sync(context.Background(), []string{"probe", "harness::send"}); err == nil {
		t.Fatal("no error")
	}
	if got := registered(b); !slices.Equal(got, []string{"harness::send", "probe::echo"}) {
		t.Fatalf("after a failed read registered %v", got)
	}
}

// A known copy whose internal flag flipped is read again, so the status goes by the new flag.
func TestSyncRereadsAFlippedInternalFlag(t *testing.T) {
	rows := []map[string]any{internal(row("probe::grant", "local", "probe"))}
	b := newBridge(t, engine(&rows))
	if err := b.Sync(context.Background(), []string{"probe", "probe::grant"}); err != nil {
		t.Fatal(err)
	}
	rows[0] = row("probe::grant", "local", "probe")
	if err := b.Sync(context.Background(), []string{"probe"}); err != nil {
		t.Fatal(err)
	}
	if got := b.Shared(); len(got) != 1 || !slices.Equal(got[0].Functions, []string{"probe::grant"}) {
		t.Fatalf("shared %+v", got)
	}
}

func TestCatalogMirrorKeepsInternal(t *testing.T) {
	rows := []map[string]any{}
	local := engine(&rows)
	b := newBridge(t, local)
	conns := mirrorsOf(b)
	b.SetCatalog(node.Catalog{Rev: "r1", Functions: []node.CatalogEntry{{ID: "harness::send", Owner: "desk", Internal: true}, {ID: "shell::exec", Owner: "hub"}}})
	if o := conns["desk"].opts["harness::send"]; string(o.Metadata) != `{"internal":true}` {
		t.Fatalf("internal mirror options %+v", o)
	}
	if o := conns["hub"].opts["shell::exec"]; o.Metadata != nil {
		t.Fatalf("mirror options %+v", o)
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
		s := b.Shared()
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
	conns := mirrorsOf(b)
	b.SetCatalog(node.Catalog{Rev: "r1", Functions: []node.CatalogEntry{{ID: "shell::exec", Owner: "hub"}}})
	hubMirror := conns["hub"]
	waitUntil(t, "shared after two rejections", func() bool {
		s := b.Shared()
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
	if b.CatalogRev() != "r1" || len(conns) != 1 || conns["hub"] != hubMirror || hubMirror.closed || len(b.Remote()) != 1 {
		t.Fatalf("mirrors after reconnects: rev %q, %d connections, hub mirror closed %v", b.CatalogRev(), len(conns), hubMirror.closed)
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

func TestDropMachineBound(t *testing.T) {
	for _, c := range []struct{ id, in, want string }{
		{"shell::exec", `{"command":"ls","fs_scope":{"root":"/hub"},"caller_fs_root":"/hub","caller_session_id":"s1"}`, `{"command":"ls"}`},
		{"a2ui::surface::get", `{"id":"x","session_id":"s1"}`, `{"id":"x"}`},
		{"ade::working-directory::propose", `{"path":"/x","session_id":"s1","turn_id":"t1"}`, `{"path":"/x"}`},
		{"session::messages", `{"session_id":"s1","turn_id":"t1"}`, `{"session_id":"s1","turn_id":"t1"}`}, // real arguments outside a2ui and ade
		{"shell::exec", `{"b": [1, 2.50],"a":"x"}`, `{"b": [1, 2.50],"a":"x"}`},                           // nothing to drop: the same bytes
		{"shell::exec", `[{"fs_scope":1}]`, `[{"fs_scope":1}]`},
		{"fp::batch", `{"calls":[{"payload":{"fs_scope":1}}]}`, `{"calls":[{"payload":{"fs_scope":1}}]}`}, // top level only
		{"shell::exec", `null`, `null`},
		{"shell::exec", ``, ``},
	} {
		if got := string(DropMachineBound(c.id, json.RawMessage(c.in))); got != c.want {
			t.Fatalf("%s %s: got %s", c.id, c.in, got)
		}
	}
}

// The bridge entry point: a call the hub engine routed to this machine.
func TestForwardDropsMachineBoundFields(t *testing.T) {
	local := &fakeLocal{fn: func(iii.CallRequest) (json.RawMessage, error) { return json.RawMessage(`{}`), nil }}
	b := NewBridge(local, "local")
	stamped := `{"command":"ls","fs_scope":{"root":"/hub","grants":["/hub/x"],"boundary":"workspace"},"caller_fs_root":"/hub","caller_session_id":"s1"}`
	if _, err := b.forward("shell::exec")(context.Background(), json.RawMessage(stamped)); err != nil {
		t.Fatal(err)
	}
	if got := string(local.called("shell::exec")[0].Payload); got != `{"command":"ls"}` {
		t.Fatalf("the local engine got %s", got)
	}
}

func TestForwardHidesThisMachinesFolderRequest(t *testing.T) {
	local := &fakeLocal{fn: func(iii.CallRequest) (json.RawMessage, error) {
		return nil, &iii.InvocationError{Code: "S215", Message: `path is outside the jail filesystem_access_request={"v":1,"requested_root":"/Users/me","attempted_path":"x","error_code":"S215"}`}
	}}
	_, err := NewBridge(local, "local").forward("shell::exec")(context.Background(), json.RawMessage(`{}`))
	var ie *iii.InvocationError
	if !errors.As(err, &ie) || ie.Code != "S215" || ie.Message != "path is outside the jail" {
		t.Fatalf("error %v", err)
	}
}

type fakeMirror struct {
	mu       sync.Mutex
	fns      map[string]iii.Handler
	opts     map[string]iii.RegisterFunctionOptions
	closed   bool
	rejected error
}

func (m *fakeMirror) RegisterFunction(id string, h iii.Handler, o ...iii.RegisterFunctionOptions) (*iii.FunctionRef, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.fns[id], m.opts[id] = h, o[0]
	return &iii.FunctionRef{ID: id}, nil
}
func (m *fakeMirror) Close() error { m.mu.Lock(); defer m.mu.Unlock(); m.closed = true; return nil }
func (m *fakeMirror) FatalError() error {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.rejected
}

// mirrorsOf makes b open fake local connections and returns them by namespace.
func mirrorsOf(b *Bridge) map[string]*fakeMirror {
	conns := map[string]*fakeMirror{}
	b.dial = func(ns string) mirrorEngine {
		conns[ns] = &fakeMirror{fns: map[string]iii.Handler{}, opts: map[string]iii.RegisterFunctionOptions{}}
		return conns[ns]
	}
	return conns
}

func TestCatalogMirrorsEachOwnerInItsNamespace(t *testing.T) {
	rows := []map[string]any{row("report::run", "busy", "report")} // a namespace this engine already uses
	b := newBridge(t, engine(&rows))                               // machine laptop, project namespace local
	conns := mirrorsOf(b)
	catalog := node.Catalog{Rev: "r1", Functions: []node.CatalogEntry{
		{ID: "shell::exec", Owner: "hub", Description: "run", RequestFormat: json.RawMessage(`{"type":"object"}`)},
		{ID: "shell::exec", Owner: "desk"},
		{ID: "coder::read-file", Owner: "desk"},
		{ID: "shell::exec", Owner: "busy"},   // a machine named like a namespace in use here
		{ID: "shell::exec", Owner: "local"},  // a machine named like this project
		{ID: "probe::echo", Owner: "laptop"}, // its own
	}}
	b.SetCatalog(catalog)
	if got := slices.Sorted(maps.Keys(conns)); !slices.Equal(got, []string{"desk", "hub"}) {
		t.Fatalf("local connections in %v", got)
	}
	if got := slices.Sorted(maps.Keys(conns["desk"].fns)); !slices.Equal(got, []string{"coder::read-file", "shell::exec"}) {
		t.Fatalf("desk mirrors %v", got)
	}
	if o := conns["hub"].opts["shell::exec"]; o.Description != "run" || string(o.RequestFormat) != `{"type":"object"}` || o.Metadata != nil {
		t.Fatalf("mirror options %+v", o)
	}
	want := []RemoteStatus{
		{Machine: "hub", Functions: []string{"shell::exec"}},
		{Machine: "busy", Functions: []string{}, Path: "hub", Skipped: "namespace busy is already in use on this engine"},
		{Machine: "desk", Functions: []string{"coder::read-file", "shell::exec"}, Path: "hub"},
		{Machine: "local", Functions: []string{}, Path: "hub", Skipped: "namespace local is already in use on this engine"},
	}
	if got := b.Remote(); b.CatalogRev() != "r1" || !reflect.DeepEqual(got, want) {
		t.Fatalf("remote %+v rev %q", got, b.CatalogRev())
	}
	var ie *iii.InvocationError
	if _, err := conns["hub"].fns["shell::exec"](context.Background(), json.RawMessage(`{}`)); !errors.As(err, &ie) || ie.Code != "mesh_offline" {
		t.Fatalf("offline call: %v", err)
	}
	// The same catalog opens nothing new.
	desk := conns["desk"]
	b.SetCatalog(catalog)
	if conns["desk"] != desk {
		t.Fatal("an unchanged owner got a new connection")
	}
	// An owner that leaves the catalog closes its connection; one that stays keeps it.
	b.SetCatalog(node.Catalog{Rev: "r2", Functions: catalog.Functions[:1]})
	if !desk.closed || conns["hub"].closed || len(b.Remote()) != 1 {
		t.Fatalf("after desk left: desk closed %v, hub closed %v, remote %v", desk.closed, conns["hub"].closed, b.Remote())
	}
	// A namespace that comes into use under a mirrored owner closes that owner's connection too.
	catalog.Rev = "r3"
	b.SetCatalog(catalog) // desk is back, on a new connection
	back := conns["desk"]
	rows = append(rows, row("report::run", "desk", "report"))
	catalog.Rev = "r4"
	b.SetCatalog(catalog)
	if back == desk || !back.closed {
		t.Fatalf("a namespace now in use keeps its mirror: reopened %v, closed %v", back != desk, back.closed)
	}
	if got := b.Remote(); len(got) != 4 || got[2].Machine != "desk" || got[2].Skipped != "namespace desk is already in use on this engine" || len(got[2].Functions) != 0 {
		t.Fatalf("remote %+v", got)
	}
	// A rename drops the catalog and closes every connection.
	if err := b.SetRuntime(context.Background(), node.Runtime{Node: "work", Namespace: "hub", URL: "ws://127.0.0.1:1"}); err != nil || !conns["hub"].closed || b.CatalogRev() != "" || len(b.Remote()) != 0 {
		t.Fatalf("a changed runtime keeps mirrors: %v, rev %q", err, b.CatalogRev())
	}
}

// A listing that fails leaves the catalog as it was and fails catalog::apply, so the hub sends this one again.
func TestCatalogWaitsForTheNamespaceListing(t *testing.T) {
	local := &fakeLocal{fn: func(iii.CallRequest) (json.RawMessage, error) { return nil, errors.New("engine away") }}
	b := newBridge(t, local)
	conns := mirrorsOf(b)
	err := b.SetCatalog(node.Catalog{Rev: "r1", Functions: []node.CatalogEntry{{ID: "shell::exec", Owner: "desk"}}})
	if err == nil || b.CatalogRev() != "" || len(conns) != 0 {
		t.Fatalf("mirrored without knowing the namespaces in use: %v, rev %q, %d connections", err, b.CatalogRev(), len(conns))
	}
}

// Two mesh clients on one engine share the mirror worker name, which the engine
// leases per namespace: a namespace the other one mirrors in is in use here.
func TestCatalogSkipsAnotherClientsMirrors(t *testing.T) {
	rows := []map[string]any{row("shell::exec", "hub", "mesh-mirror")} // another client's mirror
	b := newBridge(t, engine(&rows))
	conns := mirrorsOf(b)
	catalog := node.Catalog{Rev: "r1", Functions: []node.CatalogEntry{{ID: "shell::exec", Owner: "hub"}, {ID: "shell::exec", Owner: "desk"}}}
	if err := b.SetCatalog(catalog); err != nil {
		t.Fatal(err)
	}
	if got := slices.Sorted(maps.Keys(conns)); !slices.Equal(got, []string{"desk"}) {
		t.Fatalf("local connections in %v", got)
	}
	// Its own mirror rows keep the namespace its own.
	rows = append(rows, row("shell::exec", "desk", "mesh-mirror"))
	desk := conns["desk"]
	catalog.Rev = "r2"
	if err := b.SetCatalog(catalog); err != nil || conns["desk"] != desk || desk.closed {
		t.Fatalf("its own mirror counted as in use: %v, closed %v", err, desk.closed)
	}
	// A connection the engine rejected holds nothing: the rows there are another client's.
	desk.rejected = errors.New("WORKER_NAMESPACE_CONFLICT")
	catalog.Rev = "r3"
	if err := b.SetCatalog(catalog); err != nil || !desk.closed || conns["desk"] != desk {
		t.Fatalf("a rejected mirror stays: %v, closed %v, redialed %v", err, desk.closed, conns["desk"] != desk)
	}
	want := []RemoteStatus{
		{Machine: "hub", Functions: []string{}, Skipped: "namespace hub is already in use on this engine"},
		{Machine: "desk", Functions: []string{}, Path: "hub", Skipped: "namespace desk is already in use on this engine"},
	}
	if got := b.Remote(); !reflect.DeepEqual(got, want) {
		t.Fatalf("remote %+v", got)
	}
	// Once the namespace is free again, the next catalog dials it again.
	rows = rows[:1]
	catalog.Rev = "r4"
	if err := b.SetCatalog(catalog); err != nil || conns["desk"] == desk || conns["desk"].closed {
		t.Fatalf("a free namespace is not mirrored again: %v", err)
	}
}

// The hub sends no catalog while its revision stands, so the Sync that a local
// function change runs is what mirrors again an owner the engine rejected.
func TestSyncMirrorsARejectedOwnerAgain(t *testing.T) {
	rows := []map[string]any{}
	local := engine(&rows)
	b := newBridge(t, local)
	conns := mirrorsOf(b)
	if err := b.SetCatalog(node.Catalog{Rev: "r1", Functions: []node.CatalogEntry{{ID: "shell::exec", Owner: "desk"}}}); err != nil {
		t.Fatal(err)
	}
	// A healthy mirror reads nothing more.
	lists := len(local.called("engine::functions::list"))
	if err := b.Sync(context.Background(), nil); err != nil || len(local.called("engine::functions::list")) != lists {
		t.Fatalf("a healthy mirror read the listing again: %v", err)
	}
	// Another client on this engine won the name after the catalog was applied: its rows hold the namespace.
	lost := conns["desk"]
	lost.rejected = errors.New("WORKER_NAMESPACE_CONFLICT")
	rows = append(rows, row("shell::exec", "desk", "mesh-mirror"))
	if err := b.Sync(context.Background(), nil); err != nil || !lost.closed || conns["desk"] != lost {
		t.Fatalf("a rejected mirror under a held name: %v, closed %v, redialed %v", err, lost.closed, conns["desk"] != lost)
	}
	if got := b.Remote(); len(got) != 1 || got[0].Skipped != "namespace desk is already in use on this engine" {
		t.Fatalf("remote %+v", got)
	}
	// A listing that fails keeps the owner out and fails the Sync, which the client retries.
	rows = rows[:0]
	list := local.fn
	local.fn = func(iii.CallRequest) (json.RawMessage, error) { return nil, errors.New("engine away") }
	if err := b.Sync(context.Background(), nil); err == nil || conns["desk"] != lost {
		t.Fatalf("mirrored without knowing the namespaces in use: %v", err)
	}
	local.fn = list
	// The holder released the name: the next Sync dials the owner again, on the catalog it holds.
	if err := b.Sync(context.Background(), nil); err != nil || conns["desk"] == lost || conns["desk"].closed || conns["desk"].fns["shell::exec"] == nil {
		t.Fatalf("a free namespace is not mirrored again: %v", err)
	}
	// A rejection whose holder is gone before the Sync is dialed again at once.
	lost = conns["desk"]
	lost.rejected = errors.New("WORKER_NAMESPACE_CONFLICT")
	if err := b.Sync(context.Background(), nil); err != nil || !lost.closed || conns["desk"] == lost || conns["desk"].fns["shell::exec"] == nil {
		t.Fatalf("a rejected mirror in a free namespace stays down: %v", err)
	}
	if got := b.Remote(); b.CatalogRev() != "r1" || len(got) != 1 || got[0].Skipped != "" {
		t.Fatalf("remote %+v rev %q", got, b.CatalogRev())
	}
}

// A catalog whose namespaces were still being read when the runtime changed,
// the bridge closed or the catalog was cleared is not installed: the hub sends it again.
func TestCatalogReadAcrossAChangeIsNotInstalled(t *testing.T) {
	for name, change := range map[string]func(*Bridge){
		"stop": func(b *Bridge) { _ = b.SetRuntime(context.Background(), node.Runtime{}) },
		"rename": func(b *Bridge) {
			_ = b.SetRuntime(context.Background(), node.Runtime{Node: "work", Namespace: "hub", URL: "ws://127.0.0.1:1"})
		},
		"close": (*Bridge).Close,
		"clear": func(b *Bridge) { _ = b.SetCatalog(node.Catalog{}) },
	} {
		t.Run(name, func(t *testing.T) {
			reading, resume := make(chan struct{}), make(chan struct{})
			b := newBridge(t, &fakeLocal{fn: func(iii.CallRequest) (json.RawMessage, error) {
				close(reading)
				<-resume
				return json.RawMessage(`{"functions":[]}`), nil
			}})
			conns := mirrorsOf(b)
			done := make(chan error, 1)
			go func() {
				done <- b.SetCatalog(node.Catalog{Rev: "r1", Functions: []node.CatalogEntry{{ID: "shell::exec", Owner: "desk"}}})
			}()
			select {
			case <-reading:
			case err := <-done:
				t.Fatalf("the catalog read no namespaces: %v", err)
			}
			change(b)
			close(resume)
			if err := <-done; err == nil || b.CatalogRev() != "" || len(conns) != 0 {
				t.Fatalf("installed a catalog read before the change: %v, rev %q, %d connections", err, b.CatalogRev(), len(conns))
			}
		})
	}
}

// A Sync whose read of the namespaces in use predates a newer catalog does not
// install over it: that catalog mirrored an owner the older read still saw held.
func TestSyncReadAcrossACatalogIsNotInstalled(t *testing.T) {
	held := []map[string]any{row("shell::exec", "desk", "mesh-mirror")} // another client's mirror holds desk
	reading, resume := make(chan struct{}), make(chan struct{})
	var lists atomic.Int32
	b := newBridge(t, &fakeLocal{fn: func(iii.CallRequest) (json.RawMessage, error) {
		switch lists.Add(1) {
		case 1: // the first catalog
		case 2: // the Sync's read
			close(reading)
			<-resume
		default: // the holder left
			return json.RawMessage(`{"functions":[]}`), nil
		}
		return json.Marshal(map[string]any{"functions": held})
	}})
	conns := mirrorsOf(b)
	catalog := node.Catalog{Rev: "r1", Functions: []node.CatalogEntry{{ID: "shell::exec", Owner: "desk"}}}
	if err := b.SetCatalog(catalog); err != nil || len(conns) != 0 {
		t.Fatalf("mirrored a held namespace: %v, %d connections", err, len(conns))
	}
	done := make(chan error, 1)
	go func() { done <- b.Sync(context.Background(), nil) }()
	select {
	case <-reading:
	case err := <-done:
		t.Fatalf("the Sync read no namespaces for a skipped owner: %v", err)
	}
	catalog.Rev = "r2"
	if err := b.SetCatalog(catalog); err != nil || conns["desk"] == nil {
		t.Fatalf("a free namespace is not mirrored: %v", err)
	}
	close(resume)
	if err := <-done; err == nil || conns["desk"].closed || b.Remote()[0].Skipped != "" {
		t.Fatalf("installed a read older than the catalog: %v, closed %v, remote %+v", err, conns["desk"].closed, b.Remote())
	}
}

// A mirror is never published back: it lives in its owner's namespace, never in the project's.
func TestSyncNeverPublishesAMirror(t *testing.T) {
	rows := []map[string]any{row("shell::exec", "hub", "mesh-mirror"), row("probe::echo", "local", "probe")}
	b := newBridge(t, engine(&rows))
	if err := b.Sync(context.Background(), []string{"probe", "shell::exec"}); err != nil {
		t.Fatal(err)
	}
	if got := registered(b); !slices.Equal(got, []string{"probe::echo"}) {
		t.Fatalf("published %v", got)
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
		conns := mirrorsOf(b)
		b.SetCatalog(node.Catalog{Rev: "r1", Functions: []node.CatalogEntry{{ID: "coder::read-file", Owner: "desk"}}})
		waitUntil(t, "connected", func() bool {
			b.mu.Lock()
			defer b.mu.Unlock()
			return b.online != nil && b.online.Err() == nil
		})
		start := time.Now()
		ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second) // the SDK alone would wait 620 s
		defer cancel()
		out, err := conns["desk"].fns["coder::read-file"](ctx, json.RawMessage(`{"path":"a"}`))
		f := <-calls
		var data any
		json.Unmarshal(f["data"], &data)
		if string(f["namespace"]) != `"hub"` || !reflect.DeepEqual(data, map[string]any{"owner": "desk", "function_id": "coder::read-file", "payload": map[string]any{"path": "a"}}) {
			t.Fatalf("hub got %s", mustMarshal(f))
		}
		var ie *iii.InvocationError
		switch {
		case !drop && (err != nil || string(out.(json.RawMessage)) != `{"got":{"function_id":"coder::read-file","owner":"desk","payload":{"path":"a"}}}`):
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

// The bridge registers in the machine's namespace; what it calls on the hub names the hub's.
func TestBridgeDeclaresItsMachineNamespace(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	frames := make(chan map[string]json.RawMessage, 16)
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
			if json.Unmarshal(raw, &f) == nil && string(f["type"]) == `"invokefunction"` {
				frames <- f
			}
		}
	}))
	t.Cleanup(hub.Close)
	b := NewBridge(engine(&[]map[string]any{}), "local")
	defer b.Close()
	if err := b.SetRuntime(context.Background(), node.Runtime{Node: "laptop", Namespace: "proj", URL: "ws" + strings.TrimPrefix(hub.URL, "http")}); err != nil {
		t.Fatal(err)
	}
	waitUntil(t, "connected", func() bool { return isOnline(b) })
	if !b.Announce() {
		t.Fatal("not announced")
	}
	got := map[string]map[string]json.RawMessage{}
	for got["engine::workers::register"] == nil || got["mesh::node::changed"] == nil {
		var id string
		f := nextFrame(t, frames)
		json.Unmarshal(f["function_id"], &id)
		got[id] = f
	}
	var declared struct{ Namespace string }
	json.Unmarshal(got["engine::workers::register"]["data"], &declared)
	if declared.Namespace != "laptop" || string(got["mesh::node::changed"]["namespace"]) != `"proj"` {
		t.Fatalf("declared %q, announced in %s", declared.Namespace, got["mesh::node::changed"]["namespace"])
	}
}

func mustMarshal(v any) string { b, _ := json.Marshal(v); return string(b) }

// The page reads remote from mesh::status: the hub first, by owner and function id.
func TestStatusListsRemote(t *testing.T) {
	r := &runner{bridge: newBridge(t, engine(&[]map[string]any{}))}
	mirrorsOf(r.bridge)
	r.bridge.SetCatalog(node.Catalog{Rev: "r1", Functions: []node.CatalogEntry{{ID: "probe::echo", Owner: "alpha"}, {ID: "shell::exec", Owner: "hub"}}})
	if b, _ := json.Marshal(r.status()); !strings.Contains(string(b), `"remote":[{"machine":"hub","functions":["shell::exec"]},{"machine":"alpha","functions":["probe::echo"],"path":"hub"}]`) {
		t.Fatalf("status %s", b)
	}
}
