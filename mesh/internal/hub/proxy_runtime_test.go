package hub

import (
	"context"
	"encoding/json"
	"errors"
	"net/netip"
	"reflect"
	"testing"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/gateway"
)

type proxyRPC struct {
	applyErr   error
	applied    map[string]any
	applyCalls int
	ack        json.RawMessage
	t          *testing.T
	value      map[string]any
	err        error
	writes     int
}

func (f *proxyRPC) Trigger(_ context.Context, req iii.TriggerRequest) (json.RawMessage, error) {
	f.t.Helper()
	var p map[string]any
	if err := json.Unmarshal(req.Data, &p); err != nil {
		f.t.Fatal(err)
	}
	if req.FunctionID == "rbac-proxy::on-config-change" {
		if req.Namespace != "proj" {
			f.t.Fatalf("proxy refresh namespace: %s", req.Namespace)
		}
		f.applyCalls++
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
	if req.Namespace != "default" || p["id"] != "rbac-proxy" {
		f.t.Fatalf("proxy config routing: %+v %s", req, p)
	}
	if f.err != nil {
		return nil, f.err
	}
	switch req.FunctionID {
	case "configuration::get":
		if p["raw"] != true {
			f.t.Fatal("proxy fetch expands environment")
		}
	case "configuration::ensure":
		f.writes++
		if f.value == nil {
			f.value = p["initial_value"].(map[string]any)
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
	if _, err := h.onConfigChange(t.Context(), f, ConfigChangeEvent{ID: "rbac-proxy"}); err == nil {
		t.Fatal("foreign ownership accepted")
	}
	if h.MeshGateway.Status() == "ok" || h.proxyHealth() != "owned by other::auth" || !reflect.DeepEqual(f.value, foreign) || f.writes != 1 {
		t.Fatal("foreign ownership was overwritten or left active")
	}
	waitInvalid(t, h.Sessions, key)
	f.value = nil
	if _, err := h.onConfigChange(t.Context(), f, ConfigChangeEvent{ID: "rbac-proxy"}); err != nil {
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
