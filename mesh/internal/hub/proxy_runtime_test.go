package hub

import (
	"context"
	"encoding/json"
	"errors"
	"net/netip"
	"reflect"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/gateway"
)

type proxyRPC struct {
	mu         sync.Mutex // the hub's loop calls while a test changes the answers
	callErr    error      // fails the rbac-proxy call itself
	writeErr   error      // fails configuration::set
	gets       int        // configuration::list calls: one for each pass
	id         string     // the entry rbac-proxy registered, "rbac-proxy" when empty
	missing    bool       // rbac-proxy has registered no entry yet
	applyErr   error
	applied    map[string]any
	applyCalls int
	ack        json.RawMessage
	t          *testing.T
	value      map[string]any
	err        error
	writes     int
}

func (f *proxyRPC) set(change func()) { f.mu.Lock(); defer f.mu.Unlock(); change() }
func (f *proxyRPC) passes() int       { f.mu.Lock(); defer f.mu.Unlock(); return f.gets }
func (f *proxyRPC) entry() string {
	if f.id == "" {
		return "rbac-proxy"
	}
	return f.id
}

func (f *proxyRPC) Call(_ context.Context, req iii.CallRequest) (json.RawMessage, error) {
	f.t.Helper()
	f.mu.Lock()
	defer f.mu.Unlock()
	var p map[string]any
	if err := json.Unmarshal(req.Payload, &p); err != nil {
		f.t.Fatal(err)
	}
	if req.FunctionID == "rbac-proxy::on-config-change" {
		if req.Namespace != "proj" || p["id"] != f.entry() {
			f.t.Fatalf("proxy refresh routing: %s %s", req.Namespace, req.Payload)
		}
		f.applyCalls++
		if f.callErr != nil {
			return nil, f.callErr
		}
		if f.ack != nil {
			return f.ack, nil
		}
		if f.applyErr == nil {
			b, _ := json.Marshal(f.value)
			json.Unmarshal(b, &f.applied)
		}
		result := map[string]any{"ok": f.applyErr == nil, "applied_config": f.applied, "bound_host": f.applied["host"], "bound_port": f.applied["port"]}
		if f.applyErr != nil {
			result["error"] = f.applyErr.Error()
		}
		return json.Marshal(result)
	}
	if req.FunctionID == "configuration::list" && req.Namespace == "default" {
		f.gets++
		if f.err != nil {
			return nil, f.err
		}
		entries := []any{map[string]any{"id": "mesh", "metadata": map[string]any{"ui_form": "mesh"}}}
		if !f.missing {
			entries = append(entries, map[string]any{"id": f.entry(), "metadata": map[string]any{"ui_form": "rbac-proxy"}})
		}
		return json.Marshal(map[string]any{"configurations": entries})
	}
	if req.Namespace != "default" || p["id"] != f.entry() {
		f.t.Fatalf("proxy config routing: %+v %s", req, p)
	}
	if f.err != nil {
		return nil, f.err
	}
	if req.FunctionID != "configuration::get" && f.writeErr != nil {
		return nil, f.writeErr
	}
	switch req.FunctionID {
	case "configuration::get":
		if p["raw"] != true {
			f.t.Fatal("proxy fetch expands environment")
		}
	case "configuration::set":
		f.writes++
		f.value = p["value"].(map[string]any)
	default:
		f.t.Fatalf("unexpected proxy RPC: %s", req.FunctionID)
	}
	return json.Marshal(map[string]any{"value": f.value})
}
func TestProxyOwnershipLossAndRecovery(t *testing.T) {
	h := authHub()
	h.MeshGateway = gateway.New(func(netip.Addr) (string, bool) { return "laptop", true }, h.config.ProxyURL, h.Tickets)
	defer h.MeshGateway.Close()
	f := &proxyRPC{t: t}
	h.proxyClient = f
	h.SetHooks(Hooks{})
	if err := h.reconcileProxy(t.Context()); err == nil || h.MeshGateway.Status() == "ok" {
		t.Fatal("activated before auth registration")
	}
	h.proxyReady = true
	if err := h.reconcileProxy(t.Context()); err != nil {
		t.Fatal(err)
	}
	if h.MeshGateway.Status() != "ok" || h.proxyHealth() != "ok" || f.writes != 1 {
		t.Fatalf("startup claim: %s %s writes=%d", h.MeshGateway.Status(), h.proxyHealth(), f.writes)
	}
	key := h.Sessions.Open("laptop")
	f.value = map[string]any{"rbac": map[string]any{"auth_function_id": "other::auth"}}
	foreign := f.value
	if _, err := h.onConfigChange(t.Context(), f, proxyEvent(t, "rbac-proxy")); err == nil {
		t.Fatal("foreign ownership accepted")
	}
	if h.MeshGateway.Status() == "ok" || h.proxyHealth() != "owned by other::auth" || !reflect.DeepEqual(f.value, foreign) || f.writes != 1 {
		t.Fatal("foreign ownership was overwritten or left active")
	}
	waitInvalid(t, h.Sessions, key)
	f.value = nil
	if _, err := h.onConfigChange(t.Context(), f, proxyEvent(t, "rbac-proxy")); err != nil {
		t.Fatal(err)
	}
	if h.MeshGateway.Status() != "ok" {
		t.Fatal("ownership recovery did not reactivate")
	}
	f.err = errors.New("config unavailable")
	if err := h.reconcileProxy(t.Context()); err == nil || h.MeshGateway.Status() == "ok" {
		t.Fatal("config failure left gateway active")
	}
	f.err = errors.New("ok")
	if err := h.reconcileProxy(t.Context()); err == nil || h.MeshGateway.Status() == "ok" {
		t.Fatal("error text activated gateway")
	}
	f.err = nil
	if err := h.reconcileProxy(t.Context()); err != nil || h.MeshGateway.Status() != "ok" {
		t.Fatal("refresh could not retry")
	}
	f.value["middleware_function_id"] = "other::middleware"
	if err := h.reconcileProxy(t.Context()); err == nil || h.MeshGateway.Status() == "ok" {
		t.Fatal("invalid owned config enabled gateway")
	}
}
func TestGrantChangeClosesSessions(t *testing.T) {
	h := authHub()
	h.MeshGateway = gateway.New(func(netip.Addr) (string, bool) { return "laptop", true }, h.config.ProxyURL, h.Tickets)
	defer h.MeshGateway.Close()
	changes, revokes, closes := 0, 0, 0
	h.SetHooks(Hooks{ConfigChanged: func(context.Context, Config, Config) { changes++ }, RevokeNode: func(string) { revokes++ }, Close: func() error { closes++; return nil }})
	a, b := h.Sessions.Open("laptop"), h.Sessions.Open("desktop")
	h.revoke("laptop")
	waitInvalid(t, h.Sessions, a)
	if _, ok := h.Sessions.Valid(b); !ok {
		t.Fatal("revoked sibling node")
	}
	h.SetGrantDefaults(GrantDefaults{Allow: []string{"shell::exec"}})
	waitInvalid(t, h.Sessions, b)
	key := h.Sessions.Open("laptop")
	h.SetGrantDefaults(GrantDefaults{Allow: []string{"shell::exec"}})
	if _, ok := h.Sessions.Valid(key); !ok {
		t.Fatal("unchanged defaults revoke sessions")
	}
	f := &proxyRPC{t: t}
	h.proxyClient = f
	h.proxyReady = true
	old, next := h.Config(), h.Config()
	next.ProxyURL += "/new"
	h.hooks.ConfigChanged(t.Context(), old, next)
	if h.MeshGateway.Status() != "ok" || changes != 1 || revokes != 1 {
		t.Fatal("proxy change reconciliation lost external hooks")
	}
	h.hooks.Close()
	waitInvalid(t, h.Sessions, key)
	if closes != 1 {
		t.Fatal("lost close callback")
	}
}

func TestProxyRequiresAppliedOwnership(t *testing.T) {
	h := authHub()
	h.MeshGateway = gateway.New(func(netip.Addr) (string, bool) { return "laptop", true }, h.config.ProxyURL, h.Tickets)
	defer h.MeshGateway.Close()
	f := &proxyRPC{t: t, applyErr: errors.New("address already in use")}
	h.proxyClient = f
	h.proxyReady = true
	if err := h.reconcileProxy(t.Context()); err == nil || h.MeshGateway.Status() == "ok" {
		t.Fatal("stored ownership activated before consumer application")
	}
	if f.applyCalls != 1 {
		t.Fatal("consumer was not called")
	}
	writes := f.writes
	f.applyErr = nil
	if err := h.reconcileProxy(t.Context()); err != nil || h.MeshGateway.Status() != "ok" {
		t.Fatalf("unchanged desired config did not recover: %v", err)
	}
	if f.writes != writes || f.applyCalls != 2 {
		t.Fatal("recovery did not retry unchanged config through consumer")
	}
	for _, ack := range []string{`{}`, `{"ok":true}`, `{"ok":true,"applied_config":{},"bound_host":"127.0.0.1","bound_port":49200}`, `{"ok":false,"error":"failed apply"}`} {
		f.ack = json.RawMessage(ack)
		if err := h.reconcileProxy(t.Context()); err == nil || h.MeshGateway.Status() == "ok" {
			t.Fatalf("invalid applied ack activated: %s", ack)
		}
	}
	f.ack = nil
	h.Client = iii.New("ws://127.0.0.1:59999")
	defer h.Client.Close()
	if err := h.reconcileProxy(t.Context()); err == nil {
		t.Fatal("operator mesh config for another engine accepted")
	}
	f.value = nil
	if err := h.reconcileProxy(t.Context()); err != nil {
		t.Fatal(err)
	}
	if f.value["engine_url"] != "ws://127.0.0.1:59999" {
		t.Fatal("new claim did not follow actual Hub bus")
	}
	// A stale acknowledgement from an earlier successfully applied listener cannot activate.
	stale := map[string]any{"ok": true, "applied_config": f.applied, "bound_host": "0.0.0.0", "bound_port": 49200}
	f.ack, _ = json.Marshal(stale)
	if err := h.reconcileProxy(t.Context()); err == nil {
		t.Fatal("stale bound listener acknowledged")
	}
}

func TestProxyClaimUsesConfiguredLoopbackEndpoint(t *testing.T) {
	h := authHub()
	h.config.ProxyURL = "ws://127.0.0.1:50123"
	f := &proxyRPC{t: t}
	h.proxyClient = f
	h.proxyReady = true
	if err := h.reconcileProxy(t.Context()); err != nil {
		t.Fatal(err)
	}
	if f.value["port"] != float64(50123) {
		t.Fatalf("claimed wrong endpoint: %+v", f.value)
	}
	for _, endpoint := range []string{"ws://192.0.2.1:50123", "wss://127.0.0.1:50123", "ws://127.0.0.1:50123/path"} {
		h.config.ProxyURL = endpoint
		if err := h.reconcileProxy(t.Context()); err == nil {
			t.Fatalf("unsupported endpoint accepted: %s", endpoint)
		}
	}
	h.config.ProxyURL = "ws://127.0.0.1:50124"
	if err := h.reconcileProxy(t.Context()); err == nil {
		t.Fatal("existing different owned listener silently accepted")
	}
	if f.value["port"] != float64(50123) {
		t.Fatal("rewrote operator-owned listener")
	}
}

func TestLegacyProxyUpgradeNeverRepairsEndpointsOrConflicts(t *testing.T) {
	for _, tc := range []struct {
		name   string
		mutate func(map[string]any)
		allow  bool
	}{
		{"owned hook", func(map[string]any) {}, true},
		{"foreign bus", func(c map[string]any) { c["engine_url"] = "ws://127.0.0.1:59998" }, false},
		{"foreign listener", func(c map[string]any) { c["port"] = float64(59998) }, false},
		{"conflicting hook", func(c map[string]any) {
			c["rbac"].(map[string]any)["on_observer_registration_function_id"] = "other::hook"
		}, false},
		{"exposed shared grants", func(c map[string]any) { c["rbac"].(map[string]any)["expose_functions"] = []any{"state::*"} }, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			h := authHub()
			current := map[string]any{"host": "127.0.0.1", "port": float64(49200), "engine_url": "ws://127.0.0.1:49134", "middleware_function_id": "mesh::middleware", "operator_limit": float64(7), "rbac": map[string]any{"auth_function_id": "mesh::auth", "on_trigger_registration_function_id": "mesh::on-trigger-registration"}}
			tc.mutate(current)
			before, _ := json.Marshal(current)
			f := &proxyRPC{t: t, value: current}
			h.proxyClient = f
			h.proxyReady = true
			err := h.reconcileProxy(t.Context())
			if (err == nil) != tc.allow {
				t.Fatalf("allow=%v err=%v", tc.allow, err)
			}
			if tc.allow {
				if f.writes != 1 || f.applyCalls != 1 || f.value["operator_limit"] != float64(7) {
					t.Fatal("owned upgrade not applied exactly once")
				}
				return
			}
			after, _ := json.Marshal(f.value)
			if f.writes != 0 || f.applyCalls != 0 || string(before) != string(after) {
				t.Fatal("conflicting legacy config changed")
			}
		})
	}
}

// within waits up to 5 s for ok.
func within(t *testing.T, what string, ok func() bool) {
	t.Helper()
	for deadline := time.Now().Add(5 * time.Second); !ok(); time.Sleep(time.Millisecond) {
		if time.Now().After(deadline) {
			t.Fatal(what)
		}
	}
}

func TestRetryWaitsDoubleToAMinuteAndStartOver(t *testing.T) {
	defer func(d time.Duration) { retryMin = d }(retryMin)
	retryMin = time.Second
	var r retry
	defer r.reset()
	for _, want := range []time.Duration{time.Second, 2 * time.Second, 4 * time.Second, 8 * time.Second, 16 * time.Second, 32 * time.Second, time.Minute, time.Minute} {
		if r.again(func() {}); r.wait != want {
			t.Fatalf("wait %s, want %s", r.wait, want)
		}
	}
	if r.reset(); r.wait != 0 || r.timer.Stop() {
		t.Fatal("reset left the retry armed")
	}
	if r.again(func() {}); r.wait != time.Second {
		t.Fatalf("wait %s after a reset, want the first one", r.wait)
	}
	// One retry at a time: a pass that fails while a retry waits replaces it.
	retryMin = 50 * time.Millisecond
	r.reset()
	var first, second atomic.Int32
	r.again(func() { first.Add(1) })
	r.again(func() { second.Add(1) })
	within(t, "the retry did not run", func() bool { return second.Load() == 1 })
	if first.Load() != 0 {
		t.Fatal("a replaced retry still ran")
	}
}

// An event may have ended the failure: the pass runs at once and the wait starts over.
func TestAnEventStartsTheRetryOver(t *testing.T) {
	h, _ := tunnelHub(t, false)
	h.proxyKick = make(chan struct{}, 1)
	for name, r := range map[string]struct {
		retry *retry
		kick  func()
		woken chan struct{}
	}{"proxy": {&h.proxyRetry, h.kickProxy, h.proxyKick}, "tunnel": {&h.tunnelRetry, h.kickTunnel, h.tunnelKick}} {
		r.retry.again(func() {})
		if r.kick(); r.retry.wait != 0 || r.retry.timer.Stop() || len(r.woken) != 1 {
			t.Fatalf("%s: wait %s, %d passes asked for; want no wait and one pass", name, r.retry.wait, len(r.woken))
		}
	}
}

// proxyLoop is a hub with a healthy proxy and its loop running; first is the retry's first wait.
func proxyLoop(t *testing.T, first time.Duration) (*Hub, *proxyRPC) {
	t.Helper()
	old := retryMin
	retryMin = first
	h := authHub()
	h.proxyKick, h.tunnelKick = make(chan struct{}, 1), make(chan struct{}, 1)
	h.MeshGateway = gateway.New(func(netip.Addr) (string, bool) { return "laptop", true }, h.config.ProxyURL, h.Tickets)
	f := &proxyRPC{t: t}
	h.proxyClient, h.proxyReady = f, true
	h.SetHooks(Hooks{})
	if h.refreshProxy(t.Context()); h.proxyHealth() != "ok" {
		t.Fatalf("start: %s", h.proxyHealth())
	}
	done := make(chan struct{})
	go func() { defer close(done); h.serve(t.Context()) }()
	t.Cleanup(func() {
		<-done // the test's context ended before the cleanups
		h.proxyRetry.reset()
		h.MeshGateway.Close()
		retryMin = old
	})
	return h, f
}

func TestProxyFollowsFunctionChanges(t *testing.T) {
	h, f := proxyLoop(t, time.Hour) // no retry comes in time: only the event can run a pass
	f.set(func() { f.err = errors.New("config unavailable") })
	h.functionsChanged()
	within(t, "a function change did not run the proxy pass", func() bool { return h.proxyHealth() != "ok" && h.MeshGateway.Status() != "ok" })
	f.set(func() { f.err = nil })
	h.functionsChanged()
	within(t, "a function change did not bring the proxy back", func() bool { return h.proxyHealth() == "ok" && h.MeshGateway.Status() == "ok" })
}

// A pass that ends well, or in a failure an event ends, leaves no retry waiting and the next failure starts from the first wait.
func TestProxyPassThatEndsStopsItsRetry(t *testing.T) {
	defer func(d time.Duration) { retryMin = d }(retryMin)
	retryMin = time.Minute
	for name, end := range map[string]func(*proxyRPC){
		"it recovers": func(f *proxyRPC) { f.err = nil },
		"another function owns the entry": func(f *proxyRPC) {
			f.err, f.value = nil, map[string]any{"rbac": map[string]any{"auth_function_id": "other::auth"}}
		},
	} {
		t.Run(name, func(t *testing.T) {
			h := authHub()
			f := &proxyRPC{t: t, err: errors.New("config unavailable")}
			h.proxyClient, h.proxyReady = f, true
			defer h.proxyRetry.reset()
			if h.reconcileProxy(t.Context()); h.proxyRetry.wait != time.Minute {
				t.Fatalf("wait %s after a failure no event ends, want a retry", h.proxyRetry.wait)
			}
			end(f)
			if h.reconcileProxy(t.Context()); h.proxyRetry.wait != 0 || h.proxyRetry.timer.Stop() {
				t.Fatal("the pass left its retry waiting")
			}
		})
	}
}

func TestProxyRetriesOnlyWhatNoEventEnds(t *testing.T) {
	missing := &iii.InvocationError{Code: "function_not_found"}
	for _, tc := range []struct {
		name  string
		fail  func(*proxyRPC)
		retry bool
	}{
		{"the rbac-proxy call times out", func(f *proxyRPC) { f.callErr = iii.ErrTimeout }, true},
		{"the configuration read fails", func(f *proxyRPC) { f.err = errors.New("config unavailable") }, true},
		{"the configuration write fails", func(f *proxyRPC) { f.value, f.writeErr = nil, iii.ErrTimeout }, true},
		{"rbac-proxy could not bind", func(f *proxyRPC) { f.applyErr = errors.New("address already in use") }, true},
		{"rbac-proxy answers an unbound listener", func(f *proxyRPC) {
			f.ack, _ = json.Marshal(map[string]any{"ok": true, "applied_config": f.applied, "bound_host": f.applied["host"], "bound_port": 0})
		}, true},
		{"rbac-proxy is not connected", func(f *proxyRPC) { f.callErr = missing }, false},
		{"the configuration worker is not connected", func(f *proxyRPC) { f.err = missing }, false},
		{"another function owns the entry", func(f *proxyRPC) {
			f.value = map[string]any{"rbac": map[string]any{"auth_function_id": "other::auth"}}
		}, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			h, f := proxyLoop(t, 5*time.Millisecond)
			start := f.passes()
			f.set(func() { tc.fail(f) })
			h.kickProxy() // the only event until the failure is over
			within(t, "the event did not run a pass", func() bool { return f.passes() > start && h.proxyHealth() != "ok" })
			time.Sleep(200 * time.Millisecond)
			n := f.passes() - start
			if !tc.retry {
				if n != 1 {
					t.Fatalf("%d passes in 200 ms for a failure an event ends, want 1", n)
				}
				h.functionsChanged()
				within(t, "the event did not run a pass", func() bool { return f.passes() == start+2 })
				return
			}
			if n > 8 { // 5, 10, 20, 40, 80 ms apart: 6 passes
				t.Fatalf("%d passes in 200 ms, want a doubling wait", n)
			}
			within(t, "the failure was not tried again", func() bool { return f.passes() >= start+3 })
			f.set(func() { f.callErr, f.err, f.writeErr, f.applyErr, f.ack = nil, nil, nil, nil, nil })
			within(t, "the retry did not bring the proxy back", func() bool { return h.proxyHealth() == "ok" })
			n = f.passes()
			if time.Sleep(100 * time.Millisecond); f.passes() != n {
				t.Fatal("a pass ran after the proxy came back")
			}
		})
	}
}

func TestProxyConfigIDFollowsTheRbacProxyForm(t *testing.T) {
	form := func(id string) string { return `{"id":"` + id + `","metadata":{"ui_form":"rbac-proxy"}}` }
	stray := `{"id":"rbac-proxy","metadata":null}` // an entry an older hub created: rbac-proxy never reads it
	for _, tc := range []struct {
		name, mesh, list, want string
	}{
		{"sole entry", "mesh", form("rbac-proxy"), "rbac-proxy"},
		{"namespaced entry", "default-mesh", form("default-rbac-proxy") + "," + stray, "default-rbac-proxy"},
		{"bare mesh skips the stray entry", "mesh", stray + "," + form("default-rbac-proxy"), "default-rbac-proxy"},
		{"own namespace first", "team-mesh", form("default-rbac-proxy") + "," + form("team-rbac-proxy"), "team-rbac-proxy"},
		{"bare mesh prefers rbac-proxy", "mesh", form("default-rbac-proxy") + "," + form("rbac-proxy"), "rbac-proxy"},
		{"bare mesh otherwise takes the lowest id", "mesh", form("zeta-rbac-proxy") + "," + form("alpha-rbac-proxy"), "alpha-rbac-proxy"},
		{"otherwise the lowest id of its namespace", "default-mesh", form("default-zeta") + "," + form("alpha-rbac-proxy") + "," + form("default-proxy"), "default-proxy"},
		{"never another namespace's proxy", "beta-mesh", form("alpha-rbac-proxy"), ""},
		{"never a node's entry", "mesh", form("alpha__proxy") + "," + form("e2e-rbac-proxy"), "e2e-rbac-proxy"},
		{"never a node's entry inside its namespace", "default-mesh", form("default-x__rbac-proxy"), ""},
		{"foreign metadata shapes", "mesh", `{"id":"a","metadata":"text"},{"id":"b","metadata":{"ui_form":7}},{"id":"c"},` + form("default-rbac-proxy"), "default-rbac-proxy"},
		{"none", "mesh", `{"id":"mesh","metadata":{"ui_form":"mesh"}}`, ""},
		{"only the stray entry", "mesh", stray, ""},
	} {
		t.Run(tc.name, func(t *testing.T) {
			id, err := proxyConfigID(t.Context(), &resultRPC{out: json.RawMessage(`{"configurations":[` + tc.list + `]}`)}, tc.mesh)
			if tc.want == "" {
				if !errors.Is(err, errNoProxyEntry) {
					t.Fatalf("id %q err %v, want no entry", id, err)
				}
				return
			}
			if err != nil || id != tc.want {
				t.Fatalf("id %q err %v, want %q", id, err, tc.want)
			}
		})
	}
}

// Compose names rbac-proxy's entry <namespace>-<container>: the hub waits for it, then writes, reads and applies that entry.
func TestProxyEventForTheResolvedEntry(t *testing.T) {
	h := authHub()
	h.ConfigID = "default-mesh"
	h.MeshGateway = gateway.New(func(netip.Addr) (string, bool) { return "laptop", true }, h.config.ProxyURL, h.Tickets)
	defer h.MeshGateway.Close()
	f := &proxyRPC{t: t, id: "default-rbac-proxy", missing: true}
	h.proxyClient, h.proxyReady = f, true
	defer h.proxyRetry.reset()
	if err := h.reconcileProxy(t.Context()); !errors.Is(err, errNoProxyEntry) || h.MeshGateway.Status() == "ok" || f.writes != 0 || f.applyCalls != 0 || h.proxyRetry.wait != 0 {
		t.Fatalf("before rbac-proxy registers: err=%v writes=%d applies=%d wait=%s", err, f.writes, f.applyCalls, h.proxyRetry.wait)
	}
	f.missing = false
	if _, err := h.onConfigChange(t.Context(), f, proxyEvent(t, "default-rbac-proxy")); err != nil {
		t.Fatal(err)
	}
	if h.MeshGateway.Status() != "ok" || h.proxyHealth() != "ok" || f.writes != 1 || f.applyCalls != 1 || f.value["middleware_function_id"] != "mesh::middleware" {
		t.Fatalf("registration did not claim the entry: %s %s writes=%d applies=%d", h.MeshGateway.Status(), h.proxyHealth(), f.writes, f.applyCalls)
	}
	// A node's entry or another namespace's proxy never starts a pass: neither can be this hub's.
	for _, id := range []string{"alpha__proxy", "default-x__proxy", "other-rbac-proxy"} {
		if r, err := h.onConfigChange(t.Context(), f, proxyEvent(t, id)); err != nil || !r.OK || f.passes() != 2 {
			t.Fatalf("event for %s: ok=%v err=%v passes=%d", id, r.OK, err, f.passes())
		}
	}
	f.value["middleware_function_id"] = "other::middleware"
	if _, err := h.onConfigChange(t.Context(), f, proxyEvent(t, "default-rbac-proxy")); err == nil || h.MeshGateway.Status() == "ok" {
		t.Fatal("a change to the resolved entry was not routed to the proxy pass")
	}
}

// proxyEvent is the configuration event the engine sends for an rbac-proxy entry.
func proxyEvent(t *testing.T, id string) (e ConfigChangeEvent) {
	t.Helper()
	if err := json.Unmarshal([]byte(`{"type":"configuration","event_type":"configuration:updated","id":"`+id+`","metadata":{"ui_form":"rbac-proxy"}}`), &e); err != nil {
		t.Fatal(err)
	}
	return e
}
