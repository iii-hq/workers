package hub

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"reflect"
	"sync"
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	meshclient "github.com/iii-hq/workers/mesh/internal/client"
	"github.com/iii-hq/workers/mesh/internal/coordinator"
	"github.com/iii-hq/workers/mesh/internal/node"
)

// fakeEngine answers engine::functions::list with rows, engine::workers::list with workers and info with a schema per id.
type fakeEngine struct {
	mu      sync.Mutex
	rows    []map[string]any
	workers []map[string]any
	fail    error
	lists   int      // list calls, failed ones included
	reqs    []string // the list calls' payloads
	infos   int      // ids whose schemas were read
	onList  func()   // runs once, after the next listing is read, under mu
}

func (e *fakeEngine) Call(_ context.Context, r iii.CallRequest) (json.RawMessage, error) {
	e.mu.Lock()
	defer e.mu.Unlock()
	if r.FunctionID == "engine::functions::list" {
		e.lists++
		e.reqs = append(e.reqs, string(r.Payload))
	}
	if e.fail != nil {
		return nil, e.fail
	}
	switch r.FunctionID {
	case "engine::functions::list":
		out, err := json.Marshal(map[string]any{"functions": append([]map[string]any{}, e.rows...)})
		if e.onList != nil {
			e.onList()
			e.onList = nil
		}
		return out, err
	case "engine::workers::list":
		return json.Marshal(map[string]any{"workers": append([]map[string]any{}, e.workers...)})
	case "engine::functions::info":
		var in struct {
			IDs []string `json:"function_ids"`
		}
		json.Unmarshal(r.Payload, &in)
		e.infos += len(in.IDs)
		out := []map[string]any{}
		for _, id := range in.IDs {
			out = append(out, map[string]any{"function_id": id, "request_schema": map[string]string{"title": id}})
		}
		return json.Marshal(map[string]any{"functions": out})
	}
	return nil, errors.New("unexpected call " + r.FunctionID)
}
func (e *fakeEngine) set(rows ...map[string]any) {
	e.mu.Lock()
	e.rows = rows
	e.mu.Unlock()
}
func fn(id, namespace, worker string) map[string]any {
	return map[string]any{"function_id": id, "namespace": namespace, "worker_name": worker, "description": "about " + id}
}
func (e *fakeEngine) listCalls() int {
	e.mu.Lock()
	defer e.mu.Unlock()
	return e.lists
}

// sharedHub is a hub with the machine laptop joined on its version.
func sharedHub(t *testing.T, share ...string) (*Hub, *fakeEngine, *nodeCallRPC) {
	t.Helper()
	h, r := nodeCallHub(t)
	e := &fakeEngine{}
	h.engine = e
	h.config.Share = share
	machineVersion(t, h, h.Version)
	return h, e, r
}
func machineVersion(t *testing.T, h *Hub, version string) {
	t.Helper()
	if err := h.Coordinator.SetRuntimeMetadata("laptop", coordinator.RuntimeMetadata{OS: "linux", Arch: "amd64", Version: version}); err != nil {
		t.Fatal(err)
	}
}
func catalogOf(h *Hub) node.Catalog {
	h.mu.RLock()
	defer h.mu.RUnlock()
	return h.shared.catalog
}
func ids(c node.Catalog) []string {
	out := []string{}
	for _, e := range c.Functions {
		out = append(out, e.Owner+" "+e.ID)
	}
	return out
}

func TestCatalogRows(t *testing.T) {
	h, e, _ := sharedHub(t, "shell", "coder")
	pushes := 0
	h.emit = func([]byte) { pushes++ }
	bridge := "rbac-proxy:laptop::mesh-bridge"
	e.set(
		fn("shell::exec", "project", "shell"),                    // a hub row
		fn("shell::exec", "laptop", bridge),                      // the same id on a machine: a row of its own
		fn("coder::read-file", "laptop", bridge),                 // a machine row
		fn("mesh-node::status", "laptop", bridge),                // never published
		fn("coder::write", "laptop", "rbac-proxy:laptop::coder"), // not the bridge
		fn("shell::kill", "laptop", "shell"),                     // a hub worker in the machine's namespace is not the machine's
		fn("shell::elsewhere", "other", "shell"),                 // a namespace of no owner
		fn("approval::resolve", "project", "shell"),              // never published
	)
	if err := h.refreshShared(t.Context()); err != nil {
		t.Fatal(err)
	}
	c := catalogOf(h)
	if got := ids(c); !reflect.DeepEqual(got, []string{"laptop coder::read-file", "hub shell::exec", "laptop shell::exec"}) || c.Rev == "" {
		t.Fatalf("catalog %v rev %q", got, c.Rev)
	}
	if f := c.Functions[1]; f.Description != "about shell::exec" || string(f.RequestFormat) != `{"title":"shell::exec"}` {
		t.Fatalf("entry %+v", f)
	}
	for range 20 { // two owners of one id must not reorder between reads
		if err := h.refreshShared(t.Context()); err != nil || catalogOf(h).Rev != c.Rev {
			t.Fatalf("an unchanged read moved the revision: %v", err)
		}
	}
	want := []meshclient.SharedStatus{
		{Worker: "shell", State: "shared", Functions: []string{"shell::exec"}},
		{Worker: "coder", State: "not running", Functions: []string{}},
	}
	if s := h.Status(); !reflect.DeepEqual(s.Shared, want) || len(s.CatalogSkipped) != 0 || pushes != 1 {
		t.Fatalf("status %+v %v, %d pushes", s.Shared, s.CatalogSkipped, pushes)
	}
	rev := c.Rev
	if err := h.refreshShared(t.Context()); err != nil || catalogOf(h).Rev != rev || pushes != 1 {
		t.Fatalf("an unchanged read moved: %v %d pushes", err, pushes)
	}
	// The live list: an unshared worker leaves the status before any read.
	h.config.Share = []string{"shell"}
	if s := h.Status(); len(s.Shared) != 1 {
		t.Fatalf("status %+v", s.Shared)
	}
	machineVersion(t, h, "0.1.0")
	if err := h.refreshShared(t.Context()); err != nil {
		t.Fatal(err)
	}
	if c := catalogOf(h); !reflect.DeepEqual(ids(c), []string{"hub shell::exec"}) || c.Rev == rev {
		t.Fatalf("a machine on another version stays: %v", ids(c))
	}
	h.config.Share = nil
	if err := h.refreshShared(t.Context()); err != nil || catalogOf(h).Rev != "" || len(catalogOf(h).Functions) != 0 {
		t.Fatalf("empty catalog %+v %v", catalogOf(h), err)
	}
}

func TestCatalogSharesFunctionsByID(t *testing.T) {
	h, e, _ := sharedHub(t, "shell", "harness::send", "shell::grant", "laptop::x", "coder::run")
	internal := func(r map[string]any) map[string]any { r["metadata"] = map[string]any{"internal": true}; return r }
	e.set(
		fn("shell::exec", "project", "shell"),
		internal(fn("shell::grant", "project", "shell")), // cited
		internal(fn("shell::spawn", "project", "shell")), // a worker entry shares no internal function
		internal(fn("harness::send", "project", "harness")),
		internal(fn("harness::stop", "project", "harness")),
		fn("harness::status", "project", "harness"),
		internal(fn("coder::run", "laptop", bridgeWorker("laptop"))), // laptop shares it by id
		fn("laptop::x", "project", "other"),                          // a cited hub id that reads like a machine's old one
		fn("coder::run", "gone", bridgeWorker("gone")),               // cited, but a removed machine's row
	)
	if err := h.refreshShared(t.Context()); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(e.reqs, []string{`{"include_internal":true}`, `{"include_internal":true,"workers":["rbac-proxy:laptop::mesh-bridge"]}`}) {
		t.Fatalf("list payloads %s", e.reqs)
	}
	internalOf := map[string]bool{}
	for _, f := range catalogOf(h).Functions {
		internalOf[f.Owner+" "+f.ID] = f.Internal
	}
	want := map[string]bool{"hub harness::send": true, "laptop coder::run": true, "hub laptop::x": false, "hub shell::exec": false, "hub shell::grant": true}
	if !reflect.DeepEqual(internalOf, want) {
		t.Fatalf("catalog %v", internalOf)
	}
	status := []meshclient.SharedStatus{
		{Worker: "shell", State: "shared", Functions: []string{"shell::exec"}},
		{Worker: "harness::send", State: "shared", Functions: []string{"harness::send"}},
		{Worker: "shell::grant", State: "shared", Functions: []string{"shell::grant"}},
		{Worker: "laptop::x", State: "shared", Functions: []string{"laptop::x"}},
		{Worker: "coder::run", State: "not running", Functions: []string{}},
	}
	if s := h.Status(); !reflect.DeepEqual(s.Shared, status) {
		t.Fatalf("status %+v", s.Shared)
	}
	// Without an id entry each owner still reads internal rows, a machine's shares by id, of its listed workers only.
	h.config.Share = []string{"shell"}
	e.reqs = nil
	if err := h.refreshShared(t.Context()); err != nil || !reflect.DeepEqual(e.reqs, []string{`{"include_internal":true,"workers":["shell"]}`, `{"include_internal":true,"workers":["rbac-proxy:laptop::mesh-bridge"]}`}) {
		t.Fatalf("list payloads %s %v", e.reqs, err)
	}
	if got := ids(catalogOf(h)); !reflect.DeepEqual(got, []string{"laptop coder::run", "hub shell::exec"}) {
		t.Fatalf("catalog %v", got)
	}
}

func TestCatalogCaps(t *testing.T) {
	h, e, _ := sharedHub(t, "shell")
	rows := []map[string]any{fn("shell::exec", "project", "shell")}
	for i := range 2 * maxOwnerFunctions {
		rows = append(rows, fn(fmt.Sprintf("coder::f%03d", i), "laptop", "rbac-proxy:laptop::mesh-bridge"))
	}
	e.set(rows...)
	if err := h.refreshShared(t.Context()); err != nil {
		t.Fatal(err)
	}
	if s := h.Status(); !reflect.DeepEqual(ids(catalogOf(h)), []string{"hub shell::exec"}) || !reflect.DeepEqual(s.CatalogSkipped, []string{"laptop"}) || s.Shared[0].State != "shared" {
		t.Fatalf("catalog %v status %+v", ids(catalogOf(h)), s)
	}
	// Schemas are read per owner for one row past its cap, no more: the hub's one and laptop's first 501.
	if e.infos != maxOwnerFunctions+2 {
		t.Fatalf("read %d schemas", e.infos)
	}
	for i := range rows {
		rows[i] = fn(fmt.Sprintf("shell::f%03d", i), "project", "shell")
	}
	e.set(append(rows, fn("coder::read-file", "laptop", "rbac-proxy:laptop::mesh-bridge"))...)
	if err := h.refreshShared(t.Context()); err != nil {
		t.Fatal(err)
	}
	if s := h.Status(); !reflect.DeepEqual(ids(catalogOf(h)), []string{"laptop coder::read-file"}) || len(s.CatalogSkipped) != 0 || s.Shared[0].State != "too large" || len(s.Shared[0].Functions) != 0 {
		t.Fatalf("catalog %v status %+v", ids(catalogOf(h)), s)
	}
}

func waitCatalog(t *testing.T, h *Hub, want ...string) {
	t.Helper()
	for deadline := time.Now().Add(5 * time.Second); !reflect.DeepEqual(ids(catalogOf(h)), want); time.Sleep(10 * time.Millisecond) {
		if time.Now().After(deadline) {
			t.Fatalf("catalog %v, want %v", ids(catalogOf(h)), want)
		}
	}
}

// A failed read keeps the last catalog, still re-reads the nodes and is tried again with no event.
func TestCatalogReadFailureRetries(t *testing.T) {
	defer func(d time.Duration) { nodeRefreshDelay = d }(nodeRefreshDelay)
	nodeRefreshDelay = time.Millisecond
	h, e, r := sharedHub(t, "shell")
	r.status.Version = h.Version
	e.set(fn("shell::exec", "project", "shell"))
	h.SetHooks(Hooks{})
	h.startNodeCalls(t.Context())
	defer h.Close()
	waitCatalog(t, h, "hub shell::exec")
	e.mu.Lock()
	e.fail = errors.New("engine away")
	e.rows = append(e.rows, fn("shell::kill", "project", "shell"))
	lists := e.lists // a pass lists once per owner
	e.mu.Unlock()
	r.mu.Lock()
	statuses := r.statuses
	r.mu.Unlock()
	h.onFunctionsChange()
	for deadline := time.Now().Add(5 * time.Second); ; time.Sleep(10 * time.Millisecond) {
		r.mu.Lock()
		queued := r.statuses > statuses
		r.mu.Unlock()
		if queued && e.listCalls() > lists {
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("a failed read did not re-read the nodes")
		}
	}
	waitCatalog(t, h, "hub shell::exec")
	e.mu.Lock()
	e.fail = nil
	e.mu.Unlock()
	waitCatalog(t, h, "hub shell::exec", "hub shell::kill")
}

// Each pass replaces the retry an earlier failed one armed, so an outage keeps one retry chain.
func TestCatalogFailuresKeepOneRetry(t *testing.T) {
	h, e, _ := sharedHub(t, "shell")
	e.fail = errors.New("engine away")
	h.SetHooks(Hooks{})
	h.startNodeCalls(t.Context())
	defer h.Close()
	for e.listCalls() == 0 { // the start pass
		time.Sleep(time.Millisecond)
	}
	h.passMu.Lock() // the start pass is done once it can be held
	first := h.passRetry
	h.passMu.Unlock()
	h.refreshPass()
	if first == nil || first.Stop() {
		t.Fatal("a failed pass left the earlier retry armed")
	}
}

// A change that lands during a read runs one more pass, which sees it.
func TestCatalogChangeDuringReadRunsAnotherPass(t *testing.T) {
	defer func(d time.Duration) { nodeRefreshDelay = d }(nodeRefreshDelay)
	nodeRefreshDelay = time.Millisecond
	h, e, _ := sharedHub(t, "shell")
	h.SetHooks(Hooks{})
	h.startNodeCalls(t.Context())
	defer h.Close()
	for e.listCalls() == 0 { // the start pass
		time.Sleep(time.Millisecond)
	}
	h.passMu.Lock() // the start pass is done once it can be held
	h.passMu.Unlock()
	e.mu.Lock()
	e.onList = func() {
		e.rows = []map[string]any{fn("shell::exec", "project", "shell")}
		h.onFunctionsChange()
	}
	e.mu.Unlock()
	h.onFunctionsChange()
	waitCatalog(t, h, "hub shell::exec")
}

// A machine reaching the hub's version and a changed share list each read the catalog again.
func TestCatalogFollowsVersionAndShare(t *testing.T) {
	defer func(d time.Duration) { nodeRefreshDelay = d }(nodeRefreshDelay)
	nodeRefreshDelay = time.Millisecond
	h, e, r := sharedHub(t)
	machineVersion(t, h, "0.1.0")
	e.set(fn("shell::exec", "project", "shell"), fn("coder::read-file", "laptop", "rbac-proxy:laptop::mesh-bridge"))
	h.SetHooks(Hooks{})
	h.startNodeCalls(t.Context())
	defer h.Close()
	r.mu.Lock()
	r.status.Version = h.Version
	r.mu.Unlock()
	h.queueNode("laptop")
	waitCatalog(t, h, "laptop coder::read-file")
	for h.refreshing.Load() { // the version's pass is done: only the share list moves below
		time.Sleep(time.Millisecond)
	}
	h.passMu.Lock()
	h.passMu.Unlock()
	cfg := h.Config()
	cfg.Share = []string{"shell"}
	if err := h.ApplyConfig(t.Context(), cfg); err != nil {
		t.Fatal(err)
	}
	waitCatalog(t, h, "laptop coder::read-file", "hub shell::exec")
}
