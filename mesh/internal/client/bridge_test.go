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
}

func (f *fakeLocal) Call(_ context.Context, r iii.CallRequest) (json.RawMessage, error) {
	f.mu.Lock()
	f.calls = append(f.calls, r)
	f.mu.Unlock()
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
func newBridge(t *testing.T, local caller) *Bridge {
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
		row("bare", "local", "probe"),     // no "::"
		row("shell::run", "local", "ide"), // not shared, as from an engine ignoring the filter
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
	if f.opts.Description != "about probe::echo" || string(f.opts.Metadata) != `{"owner":"probe"}` || string(f.opts.RequestFormat) != `{"title":"probe::echo"}` || f.opts.Observable != nil {
		// Observable false would be held back by the SDK on hubs without observe.v2.
		t.Fatalf("copied options %+v", f.opts)
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
// hub's applies repeat the same runtime, so only the tick can reconnect it.
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
		t.Fatal("tick kept the rejected client")
	}
	waitUntil(t, "shared after reconnect", func() bool {
		s := b.Shared("laptop")
		return len(s) == 1 && s[0].State == "shared"
	})
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
