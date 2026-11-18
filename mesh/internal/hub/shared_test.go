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

// fakeEngine answers engine::functions::list with rows and info with a schema per id.
type fakeEngine struct {
	mu     sync.Mutex
	rows   []map[string]any
	fail   error
	lists  int    // list calls, failed ones included
	req    string // the last list call's payload
	infos  int    // ids whose schemas were read
	onList func() // runs once, after the next listing is read, under mu
}

func (e *fakeEngine) Call(_ context.Context, r iii.CallRequest) (json.RawMessage, error) {
	e.mu.Lock()
	defer e.mu.Unlock()
	if r.FunctionID == "engine::functions::list" {
		e.lists++
		e.req = string(r.Payload)
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
	h, e, _ := sharedHub(t, "shell", "coder", "laptop")
	pushes := 0
	h.emit = func([]byte) { pushes++ }
	bridge := "rbac-proxy:laptop::mesh-bridge"
	e.set(
		fn("shell::exec", "project", "shell"),                             // a hub row
		fn("shell::elsewhere", "other", "shell"),                          // another namespace
		fn("approval::resolve", "project", "shell"),                       // never published
		fn("laptop::coder::read-file", "project", bridge),                 // a machine row
		fn("laptop::mesh-node::status", "project", bridge),                // never published
		fn("laptop::shell::exec", "project", "laptop"),                    // an unnamed worker of laptop, or a hub worker under its name
		fn("laptop::coder::write", "project", "rbac-proxy:laptop::coder"), // not the bridge
	)
	if err := h.refreshShared(t.Context()); err != nil {
		t.Fatal(err)
	}
	c := catalogOf(h)
	if got := ids(c); !reflect.DeepEqual(got, []string{"laptop laptop::coder::read-file", "hub shell::exec"}) || c.Rev == "" {
		t.Fatalf("catalog %v rev %q", got, c.Rev)
	}
	if f := c.Functions[1]; f.Description != "about shell::exec" || string(f.RequestFormat) != `{"title":"shell::exec"}` {
		t.Fatalf("entry %+v", f)
	}
	want := []meshclient.SharedStatus{
		{Worker: "shell", State: "shared", Functions: []string{"hub::shell::exec"}},
		{Worker: "coder", State: "not running", Functions: []string{}},
		{Worker: "laptop", State: "name conflict", Functions: []string{}},
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
	h, e, _ := sharedHub(t, "shell", "harness::send", "shell::grant", "laptop::x", "gone::coder::run")
	internal := func(r map[string]any) map[string]any { r["metadata"] = map[string]any{"internal": true}; return r }
	e.set(
		fn("shell::exec", "project", "shell"),
		internal(fn("shell::grant", "project", "shell")), // cited
		internal(fn("shell::spawn", "project", "shell")), // a worker entry shares no internal function
		internal(fn("harness::send", "project", "harness")),
		internal(fn("harness::stop", "project", "harness")),
		fn("harness::status", "project", "harness"),
		internal(fn("laptop::coder::run", "project", bridgeWorker("laptop"))), // laptop shares it by id
		fn("laptop::x", "project", "other"),                                   // a cited hub id under a machine's name
		fn("gone::coder::run", "project", bridgeWorker("gone")),               // cited, but a removed machine's row
	)
	if err := h.refreshShared(t.Context()); err != nil {
		t.Fatal(err)
	}
	if e.req != `{"include_internal":true}` {
		t.Fatalf("list payload %s", e.req)
	}
	internalOf := map[string]bool{}
	for _, f := range catalogOf(h).Functions {
		internalOf[f.Owner+" "+f.ID] = f.Internal
	}
	want := map[string]bool{"hub harness::send": true, "laptop laptop::coder::run": true, "hub shell::exec": false, "hub shell::grant": true}
	if !reflect.DeepEqual(internalOf, want) {
		t.Fatalf("catalog %v", internalOf)
	}
	status := []meshclient.SharedStatus{
		{Worker: "shell", State: "shared", Functions: []string{"hub::shell::exec"}},
		{Worker: "harness::send", State: "shared", Functions: []string{"hub::harness::send"}},
		{Worker: "shell::grant", State: "shared", Functions: []string{"hub::shell::grant"}},
		{Worker: "laptop::x", State: "name conflict", Functions: []string{}},
		{Worker: "gone::coder::run", State: "not running", Functions: []string{}},
	}
	if s := h.Status(); !reflect.DeepEqual(s.Shared, status) {
		t.Fatalf("status %+v", s.Shared)
	}
	// Without an id entry the hub still reads internal rows, a machine's shares by id, of the listed workers only.
	h.config.Share = []string{"shell"}
	if err := h.refreshShared(t.Context()); err != nil || e.req != `{"include_internal":true,"workers":["shell","rbac-proxy:laptop::mesh-bridge"]}` {
		t.Fatalf("list payload %s %v", e.req, err)
	}
	if got := ids(catalogOf(h)); !reflect.DeepEqual(got, []string{"laptop laptop::coder::run", "hub shell::exec"}) {
		t.Fatalf("catalog %v", got)
	}
}

func TestCatalogCaps(t *testing.T) {
	h, e, _ := sharedHub(t, "shell")
	rows := []map[string]any{fn("shell::exec", "project", "shell")}
	for i := range 2 * maxOwnerFunctions {
		rows = append(rows, fn(fmt.Sprintf("laptop::coder::f%03d", i), "project", "rbac-proxy:laptop::mesh-bridge"))
	}
	e.set(rows...)
	if err := h.refreshShared(t.Context()); err != nil {
		t.Fatal(err)
	}
	if s := h.Status(); !reflect.DeepEqual(ids(catalogOf(h)), []string{"hub shell::exec"}) || !reflect.DeepEqual(s.CatalogSkipped, []string{"laptop"}) || s.Shared[0].State != "shared" {
		t.Fatalf("catalog %v status %+v", ids(catalogOf(h)), s)
	}
	// Schemas are read for one row past the cap, no more: the hub's and laptop's first 501.
	if e.infos != maxOwnerFunctions+2 {
		t.Fatalf("read %d schemas", e.infos)
	}
	for i := range rows {
		rows[i] = fn(fmt.Sprintf("shell::f%03d", i), "project", "shell")
	}
	e.set(append(rows, fn("laptop::coder::read-file", "project", "rbac-proxy:laptop::mesh-bridge"))...)
	if err := h.refreshShared(t.Context()); err != nil {
		t.Fatal(err)
	}
	if s := h.Status(); !reflect.DeepEqual(ids(catalogOf(h)), []string{"laptop laptop::coder::read-file"}) || len(s.CatalogSkipped) != 0 || s.Shared[0].State != "too large" || len(s.Shared[0].Functions) != 0 {
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
	e.mu.Unlock()
	r.mu.Lock()
	statuses := r.statuses
	r.mu.Unlock()
	h.onFunctionsChange()
	for deadline := time.Now().Add(5 * time.Second); ; time.Sleep(10 * time.Millisecond) {
		r.mu.Lock()
		queued := r.statuses > statuses
		r.mu.Unlock()
		if queued && e.listCalls() >= 2 {
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

// Removing a machine reads the catalog again: an offline one leaves no function change, yet its name blocks a hub share.
func TestRemovingAMachineFreesItsName(t *testing.T) {
	defer func(d time.Duration) { nodeRefreshDelay = d }(nodeRefreshDelay)
	nodeRefreshDelay = time.Millisecond
	h, e, r := sharedHub(t, "laptop")
	r.status.Version = h.Version
	h.nodeDetails = map[string]NodeDetails{"laptop": {NodeEntry: NodeEntry{Version: h.Version}}} // no version move: the start pass is the only one
	e.set(fn("laptop::run", "project", "laptop"))
	h.SetHooks(Hooks{})
	h.Coordinator.SetGone(h.revoke)
	h.startNodeCalls(t.Context())
	defer h.Close()
	state := func() string {
		if s := h.Status().Shared; len(s) == 1 {
			return s[0].State
		}
		return "" // before the first read
	}
	waitShare := func(want string) {
		t.Helper()
		for deadline := time.Now().Add(5 * time.Second); state() != want; time.Sleep(10 * time.Millisecond) {
			if time.Now().After(deadline) {
				t.Fatalf("share %+v, want %s", h.Status().Shared, want)
			}
		}
	}
	waitShare("name conflict")
	h.passMu.Lock() // no pass left running
	h.passMu.Unlock()
	if err := h.Coordinator.Remove("laptop"); err != nil {
		t.Fatal(err)
	}
	waitShare("shared")
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
	e.set(fn("shell::exec", "project", "shell"), fn("laptop::coder::read-file", "project", "rbac-proxy:laptop::mesh-bridge"))
	h.SetHooks(Hooks{})
	h.startNodeCalls(t.Context())
	defer h.Close()
	r.mu.Lock()
	r.status.Version = h.Version
	r.mu.Unlock()
	h.queueNode("laptop")
	waitCatalog(t, h, "laptop laptop::coder::read-file")
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
	waitCatalog(t, h, "laptop laptop::coder::read-file", "hub shell::exec")
}
