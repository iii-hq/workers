//go:build e2e

package e2e

import (
	"context"
	"encoding/json"
	"fmt"
	"sync/atomic"
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/hub"
)

// The hub follows quick-tunnel by its events alone: its functions arriving, then its on-changed.
func TestE2ETunnelFollowsQuickTunnel(t *testing.T) {
	f := newFixture(t, "default")
	tunnel := func(want, url string) func() error {
		return func() error {
			out, err := call(f.ctx, f.admin, f.ns, "mesh::status", map[string]any{})
			var status hub.StatusResult
			if err == nil {
				err = json.Unmarshal(out, &status)
			}
			if err == nil && (status.Tunnel == nil || status.Tunnel.State != want || status.Tunnel.URL != url) {
				err = fmt.Errorf("tunnel %+v", status.Tunnel)
			}
			return err
		}
	}
	cfg := f.config
	cfg.PublicURL, cfg.Tunnel = "", &hub.TunnelConfig{Enabled: true}
	mustCall(t, f.admin, "default", "configuration::set", map[string]any{"id": "mesh", "value": cfg})
	eventually(t, 5*time.Second, "the tunnel unavailable with no quick-tunnel", tunnel("unavailable", ""))

	// A stand-in for quick-tunnel, registered as the real one: on-changed first and private.
	qt := connect(t, f.ctx, f.engineURL, "quick-tunnel", f.ns)
	var snapshot atomic.Value
	snapshot.Store(map[string]any{"status": "starting", "leases": []any{map[string]any{"lease_id": "lease-1"}}})
	for _, fn := range []struct {
		id     string
		answer func() any
	}{
		{"quick-tunnel::on-changed", nil},
		{"quick-tunnel::acquire", func() any { return map[string]any{"lease_id": "lease-1"} }},
		{"quick-tunnel::release", func() any { return map[string]any{} }},
		{"quick-tunnel::status", func() any { return snapshot.Load() }}, // last: a status the hub can read means every function is there
	} {
		handler, opts := iii.Handler(iii.Identity), iii.RegisterFunctionOptions{Private: true}
		if fn.answer != nil {
			handler, opts = func(context.Context, json.RawMessage) (any, error) { return fn.answer(), nil }, iii.RegisterFunctionOptions{}
		}
		if _, err := qt.RegisterFunction(fn.id, handler, opts); err != nil {
			t.Fatal(err)
		}
	}
	eventually(t, 5*time.Second, "the hub acquires once quick-tunnel's functions arrive", tunnel("starting", ""))

	time.Sleep(time.Second) // the function-set events of the registrations end, so only on-changed can bring the URL below
	snapshot.Store(map[string]any{"status": "ready", "public_url": "https://mesh.example.com", "leases": []any{map[string]any{"lease_id": "lease-1"}}})
	if _, err := qt.Call(f.ctx, iii.CallRequest{FunctionID: "quick-tunnel::on-changed", Payload: json.RawMessage(`{"tunnel_id":"mesh"}`), Action: iii.VoidAction()}); err != nil {
		t.Fatal(err)
	}
	eventually(t, 5*time.Second, "the hub reads the tunnel again on quick-tunnel::on-changed", tunnel("ready", "https://mesh.example.com"))
}
