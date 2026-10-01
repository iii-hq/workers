package main

import (
	"context"
	"encoding/json"
	"fmt"
	"sync"
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

// This provider exercises the actual engine namespace and subscription wiring
// without launching cloudflared or publishing a test service on the Internet.
type smokeTunnelProvider struct {
	mu       sync.Mutex
	client   *iii.Client
	bindings map[string]iii.TriggerConfig
	leases   map[string]map[string]any
	status   string
	origin   any
	released []string
}

func newSmokeTunnelProvider(t *testing.T, engine, engineURL string) *smokeTunnelProvider {
	t.Helper()
	p := &smokeTunnelProvider{
		client:   iii.New(engineURL, iii.WithName("smoke-quick-tunnel"), iii.WithNamespace(smokeNamespace)),
		bindings: map[string]iii.TriggerConfig{},
		leases: map[string]map[string]any{"github-existing": {
			"lease_id": "github-existing", "consumer_id": "github:existing", "tunnel_id": "webhooks",
			"expires_at": time.Now().Add(time.Hour).UTC().Format(time.RFC3339),
		}},
		status: "starting",
	}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	if err := p.client.Connect(ctx); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = p.client.Close() })
	register := func(id string, handler iii.Handler) {
		t.Helper()
		if err := p.client.RegisterFunction(id, handler); err != nil {
			t.Fatal(err)
		}
	}
	register("quick-tunnel::status", func(_ context.Context, _ json.RawMessage) (any, error) {
		p.mu.Lock()
		defer p.mu.Unlock()
		return p.snapshot(), nil
	})
	register("quick-tunnel::acquire", func(_ context.Context, raw json.RawMessage) (any, error) {
		var req struct {
			ConsumerID string `json:"consumer_id"`
			TunnelID   string `json:"tunnel_id"`
			ExpiresAt  string `json:"expires_at"`
		}
		if err := json.Unmarshal(raw, &req); err != nil {
			return nil, err
		}
		if req.ConsumerID == "" || req.TunnelID != "webhooks" {
			return nil, fmt.Errorf("invalid lease request")
		}
		p.mu.Lock()
		defer p.mu.Unlock()
		leaseID := "lease-" + req.ConsumerID
		p.leases[leaseID] = map[string]any{"lease_id": leaseID, "consumer_id": req.ConsumerID, "tunnel_id": req.TunnelID, "expires_at": req.ExpiresAt}
		result := p.snapshot()
		result["lease_id"] = leaseID
		return result, nil
	})
	register("quick-tunnel::release", func(_ context.Context, raw json.RawMessage) (any, error) {
		var req struct {
			LeaseID string `json:"lease_id"`
		}
		if err := json.Unmarshal(raw, &req); err != nil {
			return nil, err
		}
		p.mu.Lock()
		defer p.mu.Unlock()
		_, exists := p.leases[req.LeaseID]
		if exists {
			delete(p.leases, req.LeaseID)
			p.released = append(p.released, req.LeaseID)
		}
		return map[string]bool{"released": exists}, nil
	})
	if err := p.client.RegisterTriggerType("quick-tunnel::changed", "Synthetic tunnel lifecycle", p); err != nil {
		t.Fatal(err)
	}
	smokeWait(t, "synthetic tunnel provider", func() bool {
		_, err := smokeTrigger(engine, engineURL, smokeNamespace, "quick-tunnel::status", map[string]any{})
		return err == nil
	})
	return p
}

// snapshot requires p.mu; returned maps are fresh so encoding cannot race writes.
func (p *smokeTunnelProvider) snapshot() map[string]any {
	leases := make([]any, 0, len(p.leases))
	for _, lease := range p.leases {
		copy := map[string]any{}
		for k, v := range lease {
			copy[k] = v
		}
		leases = append(leases, copy)
	}
	return map[string]any{
		"tunnel_id": "webhooks", "status": p.status, "public_url": p.origin, "generation": "test-generation", "error": nil, "leases": leases,
		"prerequisites": map[string]any{"cloudflared": map[string]any{"found": true, "path": "/test/cloudflared", "error": nil}},
	}
}
func (p *smokeTunnelProvider) RegisterTrigger(_ context.Context, cfg iii.TriggerConfig) error {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.bindings[cfg.ID] = cfg
	return nil
}
func (p *smokeTunnelProvider) UnregisterTrigger(_ context.Context, cfg iii.TriggerConfig) error {
	p.mu.Lock()
	defer p.mu.Unlock()
	delete(p.bindings, cfg.ID)
	return nil
}
func (p *smokeTunnelProvider) change(t *testing.T, state string, origin any) {
	t.Helper()
	p.mu.Lock()
	p.status = state
	p.origin = origin
	p.mu.Unlock()
	p.deliver(t, state, origin)
}
func (p *smokeTunnelProvider) deliver(t *testing.T, state string, origin any) {
	t.Helper()
	p.mu.Lock()
	bindings := make([]iii.TriggerConfig, 0, len(p.bindings))
	for _, binding := range p.bindings {
		bindings = append(bindings, binding)
	}
	p.mu.Unlock()
	if len(bindings) == 0 {
		t.Fatal("WhatsApp did not subscribe to tunnel changes")
	}
	for _, binding := range bindings {
		if binding.Namespace != smokeNamespace {
			t.Fatalf("tunnel callback escaped namespace: %s", binding.Namespace)
		}
		data, _ := json.Marshal(map[string]any{"tunnel_id": "webhooks", "status": state, "public_url": origin, "generation": "test-generation", "error": nil})
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		_, err := p.client.Trigger(ctx, iii.TriggerRequest{FunctionID: binding.FunctionID, Namespace: binding.Namespace, Data: data, Timeout: 5 * time.Second})
		cancel()
		if err != nil {
			t.Fatalf("tunnel event dispatch: %v", err)
		}
	}
}

func smokeDevelopmentTunnel(t *testing.T, engine, engineURL string, p *smokeTunnelProvider) {
	t.Helper()
	call := func(operation string) map[string]any {
		t.Helper()
		result, err := smokeTrigger(engine, engineURL, smokeNamespace, "whatsapp::cloud::tunnel::"+operation, map[string]any{})
		if err != nil {
			t.Fatal(err)
		}
		return result
	}
	if got := call("status"); got["enabled"] != false || got["webhook_url"] != nil {
		t.Fatalf("page load started a tunnel: %v", got)
	}
	smokeWait(t, "WhatsApp tunnel subscription", func() bool { p.mu.Lock(); defer p.mu.Unlock(); return len(p.bindings) > 0 })
	started := call("start")
	if started["enabled"] != true || started["webhook_url"] != nil {
		t.Fatalf("unready tunnel advertised URL: %v", started)
	}
	_ = call("start")
	p.mu.Lock()
	leaseCount := len(p.leases)
	p.mu.Unlock()
	if leaseCount != 2 {
		t.Fatalf("idempotent start lost/shared/duplicated a lease: %d", leaseCount)
	}
	p.change(t, "ready", "https://first-test.trycloudflare.com")
	want := "https://first-test.trycloudflare.com/webhooks/whatsapp-cloud/" + smokeNamespace
	if got := call("status"); got["webhook_url"] != want {
		t.Fatalf("incorrect full callback: %v", got)
	}
	p.change(t, "reconnecting", nil)
	if got := call("status"); got["webhook_url"] != nil {
		t.Fatalf("reconnecting returned stale callback: %v", got)
	}
	p.change(t, "ready", "https://second-test.trycloudflare.com")
	p.deliver(t, "ready", "https://first-test.trycloudflare.com")
	want = "https://second-test.trycloudflare.com/webhooks/whatsapp-cloud/" + smokeNamespace
	if got := call("status"); got["webhook_url"] != want {
		t.Fatalf("stale event replaced authoritative URL: %v", got)
	}
	for _, operation := range []string{"stop", "stop", "status"} {
		if got := call(operation); got["enabled"] != false || got["webhook_url"] != nil {
			t.Fatalf("stopped tunnel returned callback: %v", got)
		}
	}
	p.mu.Lock()
	_, githubPreserved := p.leases["github-existing"]
	leaseCount = len(p.leases)
	releases := len(p.released)
	p.mu.Unlock()
	if !githubPreserved || leaseCount != 1 || releases != 1 {
		t.Fatalf("stop affected other consumer or repeated release: github=%v leases=%d releases=%d", githubPreserved, leaseCount, releases)
	}
}
