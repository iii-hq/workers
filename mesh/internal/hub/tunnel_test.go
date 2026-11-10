package hub

import (
	"context"
	"encoding/json"
	"errors"
	"reflect"
	"strings"
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/coordinator"
	"github.com/iii-hq/workers/mesh/internal/join"
)

// tunnelRPC fakes quick-tunnel's acquire/status/release contract.
type tunnelRPC struct {
	calls                             []string
	payloads                          []map[string]any
	acquireErr, statusErr, releaseErr error
	state, url                        string
	dropLease                         bool
	proxy, provider, lease            string // lease: the id granted, default "lease-"+tunnel_id
	onStatus                          func()
}

func (f *tunnelRPC) leaseID(tunnelID string) string {
	if f.lease != "" {
		return f.lease
	}
	return "lease-" + tunnelID
}

func (f *tunnelRPC) Call(_ context.Context, r iii.CallRequest) (json.RawMessage, error) {
	var p map[string]any
	if err := json.Unmarshal(r.Payload, &p); err != nil {
		return nil, err
	}
	f.calls, f.payloads = append(f.calls, r.FunctionID), append(f.payloads, p)
	switch r.FunctionID {
	case "quick-tunnel::acquire":
		if f.acquireErr != nil {
			return nil, f.acquireErr
		}
		return json.Marshal(map[string]any{"lease_id": f.leaseID(p["tunnel_id"].(string)), "tunnel_id": p["tunnel_id"], "status": "starting", "generation": "1"})
	case "quick-tunnel::status":
		if f.onStatus != nil {
			f.onStatus()
		}
		if f.statusErr != nil {
			return nil, f.statusErr
		}
		leases := []map[string]any{}
		if !f.dropLease {
			l := map[string]any{"lease_id": f.leaseID(p["tunnel_id"].(string))}
			if f.proxy != "" {
				l["proxy_protocol"] = f.proxy
			}
			leases = append(leases, l)
		}
		var url any
		if f.url != "" {
			url = f.url
		}
		res := map[string]any{"tunnel_id": p["tunnel_id"], "status": f.state, "public_url": url, "error": nil, "generation": "1", "leases": leases}
		if f.provider != "" {
			res["provider"] = f.provider
		}
		return json.Marshal(res)
	case "quick-tunnel::release":
		if f.releaseErr != nil {
			return nil, f.releaseErr
		}
		return json.RawMessage(`{"released":true}`), nil
	}
	return nil, errors.New("unexpected " + r.FunctionID)
}
func (f *tunnelRPC) take() []string { c := f.calls; f.calls = nil; return c }

func tunnelHub(t *testing.T, coord bool) (*Hub, *tunnelRPC) {
	f := &tunnelRPC{state: "starting"}
	cfg := DefaultConfig()
	cfg.Tunnel = &TunnelConfig{Enabled: true}
	h := &Hub{Version: "0.1.0", Namespace: "default", ConfigID: "mesh", config: cfg, tunnelClient: f, tunnelKick: make(chan struct{}, 1)}
	if coord {
		c, err := coordinator.New(t.TempDir(), nil)
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() { c.Close() })
		h.Coordinator = c
	}
	return h, f
}
func wantCalls(t *testing.T, f *tunnelRPC, want ...string) {
	t.Helper()
	if got := f.take(); !reflect.DeepEqual(got, want) && (len(got) != 0 || len(want) != 0) {
		t.Fatalf("calls = %v, want %v", got, want)
	}
}

func TestTunnelReadyBecomesPublicURL(t *testing.T) {
	h, f := tunnelHub(t, true)
	ctx := context.Background()
	h.reconcileTunnel(ctx)
	wantCalls(t, f, "quick-tunnel::acquire", "quick-tunnel::status")
	acquire := f.payloads[0]
	expires, _ := time.Parse(time.RFC3339Nano, acquire["expires_at"].(string))
	if c := acquire["consumer_id"].(string); !strings.HasPrefix(c, "mesh:") || len(c) != 37 || acquire["tunnel_id"] != "mesh" || time.Until(expires) < 9*time.Minute {
		t.Fatalf("acquire payload %v", acquire)
	}
	if s := h.Status(); s.PublicURL != "" || s.Tunnel == nil || s.Tunnel.State != "starting" {
		t.Fatalf("not-ready status %+v %+v", s, s.Tunnel)
	}
	if _, err := h.JoinCode(ctx, struct{}{}); err == nil || err.Error() != "public_url is not set and the tunnel is not ready" {
		t.Fatalf("join before ready: %v", err)
	}

	f.state, f.url = "ready", "https://abc234.opentunnel.xyz"
	h.reconcileTunnel(ctx)
	wantCalls(t, f, "quick-tunnel::status")
	if s := h.Status(); s.PublicURL != f.url || s.Tunnel.State != "ready" || s.Tunnel.URL != f.url {
		t.Fatalf("ready status %+v %+v", s, s.Tunnel)
	}
	code, err := h.JoinCode(ctx, struct{}{})
	if err != nil || code.Code == "" {
		t.Fatalf("join code %+v %v", code, err)
	}
	issued, err := h.JoinToken(ctx, join.TokenOptions{})
	if err != nil {
		t.Fatal(err)
	}
	if env, err := join.DecodeToken(issued.Token); err != nil || env.PublicURL != f.url {
		t.Fatalf("token URL %+v %v", env, err)
	}

	f.url = "https://abc234.opentunnel.xyz/path?x=1" // never trusted into join codes
	h.reconcileTunnel(ctx)
	if s := h.Status(); s.PublicURL != "" {
		t.Fatalf("unclean tunnel URL used: %q", s.PublicURL)
	}
}

func TestTunnelExplicitPublicURLWins(t *testing.T) {
	h, f := tunnelHub(t, false)
	ctx := context.Background()
	h.config.PublicURL = "https://hub.example"
	h.reconcileTunnel(ctx)
	wantCalls(t, f)
	if s := h.Status(); s.PublicURL != "https://hub.example" || s.Tunnel.State != "overridden" {
		t.Fatalf("status %+v %+v", s, s.Tunnel)
	}

	h.config.PublicURL = ""
	f.state, f.url = "ready", "https://abc234.opentunnel.xyz"
	h.reconcileTunnel(ctx)
	wantCalls(t, f, "quick-tunnel::acquire", "quick-tunnel::status")
	h.config.PublicURL = "https://hub.example"
	h.reconcileTunnel(ctx)
	wantCalls(t, f, "quick-tunnel::release")
	if s := h.Status(); s.PublicURL != "https://hub.example" || s.Tunnel.State != "overridden" || s.Tunnel.URL != "" {
		t.Fatalf("status after override %+v %+v", s, s.Tunnel)
	}

	h.config.Tunnel = nil
	h.reconcileTunnel(ctx)
	wantCalls(t, f)
	if h.Status().Tunnel != nil {
		t.Fatal("tunnel reported while disabled")
	}
}

func TestTunnelLeaseRenewal(t *testing.T) {
	h, f := tunnelHub(t, false)
	ctx := context.Background()
	h.reconcileTunnel(ctx)
	f.take()
	h.tunnel.expires = time.Now().Add(6 * time.Minute)
	h.reconcileTunnel(ctx)
	wantCalls(t, f, "quick-tunnel::status")
	h.tunnel.expires = time.Now().Add(4 * time.Minute)
	h.reconcileTunnel(ctx)
	wantCalls(t, f, "quick-tunnel::acquire", "quick-tunnel::status")

	f.dropLease = true // quick-tunnel lost the lease
	h.reconcileTunnel(ctx)
	wantCalls(t, f, "quick-tunnel::status")
	h.reconcileTunnel(ctx)
	wantCalls(t, f, "quick-tunnel::acquire", "quick-tunnel::status")
}

func TestTunnelIDChangeReleasesThenAcquires(t *testing.T) {
	h, f := tunnelHub(t, false)
	ctx := context.Background()
	h.reconcileTunnel(ctx)
	f.take()
	h.config.Tunnel.TunnelID = "hub"
	h.reconcileTunnel(ctx)
	wantCalls(t, f, "quick-tunnel::release", "quick-tunnel::acquire", "quick-tunnel::status")
	if n := len(f.payloads); f.payloads[n-3]["lease_id"] != "lease-mesh" || f.payloads[n-2]["tunnel_id"] != "hub" {
		t.Fatalf("payloads %v", f.payloads[n-3:])
	}
}

func TestTunnelUnavailable(t *testing.T) {
	for _, tc := range []struct {
		err  error
		want string
	}{
		{iii.ErrTimeout, "quick-tunnel is not installed or not responding"},
		{&iii.InvocationError{Code: "function_not_found", Message: "quick-tunnel::acquire"}, "quick-tunnel is not installed or not responding"},
		{errors.New("invalid consumer_id or unauthorized tunnel_id"), `quick-tunnel has no target "mesh"; add "mesh: http://127.0.0.1:49180" to its targets`},
		{&iii.InvocationError{Code: "invocation_failed", Message: "expires_at must be in the future within max_lease_seconds"}, "quick-tunnel refused: expires_at must be in the future within max_lease_seconds"},
	} {
		h, f := tunnelHub(t, false)
		f.acquireErr = tc.err
		h.reconcileTunnel(context.Background())
		wantCalls(t, f, "quick-tunnel::acquire")
		if s := h.Status(); s.PublicURL != "" || s.Tunnel.State != "unavailable" || s.Tunnel.Error != tc.want {
			t.Fatalf("%v: %+v", tc.err, s.Tunnel)
		}
	}
}

// A blip in quick-tunnel's RPCs must not unpublish a tunnel that still serves.
func TestTunnelKeepsURLWhileLeaseHolds(t *testing.T) {
	h, f := tunnelHub(t, true)
	ctx := context.Background()
	f.state, f.url = "ready", "https://abc234.opentunnel.xyz"
	h.reconcileTunnel(ctx)
	f.take()

	f.statusErr = iii.ErrTimeout
	h.reconcileTunnel(ctx)
	if s := h.Status(); s.PublicURL != f.url || s.Tunnel.State != "ready" || s.Tunnel.Error != tunnelNotResponding {
		t.Fatalf("status error dropped the URL: %+v %+v", s, s.Tunnel)
	}
	h.tunnel.expires = time.Now().Add(time.Minute) // renewal due, and it fails too
	f.acquireErr = iii.ErrTimeout
	h.reconcileTunnel(ctx)
	wantCalls(t, f, "quick-tunnel::status", "quick-tunnel::acquire")
	if s := h.Status(); s.PublicURL != f.url {
		t.Fatalf("renewal error dropped the URL: %+v", s)
	}

	h.tunnel.expires = time.Now().Add(-time.Second) // lease gone: no longer trusted
	h.reconcileTunnel(ctx)
	if s := h.Status(); s.PublicURL != "" || s.Tunnel.State != "unavailable" {
		t.Fatalf("expired lease kept the URL: %+v %+v", s, s.Tunnel)
	}
}

func TestTunnelRetriesFailedRelease(t *testing.T) {
	h, f := tunnelHub(t, false)
	ctx := context.Background()
	h.reconcileTunnel(ctx)
	f.take()
	f.releaseErr = iii.ErrTimeout
	h.config.Tunnel.Enabled = false
	h.reconcileTunnel(ctx)
	wantCalls(t, f, "quick-tunnel::release")
	f.releaseErr = nil
	h.reconcileTunnel(ctx)
	h.reconcileTunnel(ctx)
	wantCalls(t, f, "quick-tunnel::release")

	h.config.Tunnel.Enabled = true // a new target waits for the old lease's release
	h.reconcileTunnel(ctx)
	f.take()
	f.releaseErr = iii.ErrTimeout
	h.config.Tunnel.TunnelID = "hub"
	h.reconcileTunnel(ctx)
	wantCalls(t, f, "quick-tunnel::release")
	f.releaseErr = nil
	h.reconcileTunnel(ctx)
	wantCalls(t, f, "quick-tunnel::release", "quick-tunnel::acquire", "quick-tunnel::status")
}

func TestTunnelChangeKicksOnlyForOwnTarget(t *testing.T) {
	h, _ := tunnelHub(t, false)
	h.onTunnelChange(TunnelChangeEvent{TunnelID: "webhooks"})
	if len(h.tunnelKick) != 0 {
		t.Fatal("kicked for another tunnel")
	}
	h.onTunnelChange(TunnelChangeEvent{TunnelID: "mesh"})
	h.onTunnelChange(TunnelChangeEvent{TunnelID: "mesh"}) // never blocks
	if len(h.tunnelKick) != 1 {
		t.Fatal("own tunnel change did not kick")
	}
}

func TestReleaseTunnelOnce(t *testing.T) {
	h, f := tunnelHub(t, false)
	h.reconcileTunnel(context.Background())
	f.take()
	h.releaseTunnel()
	h.releaseTunnel()
	wantCalls(t, f, "quick-tunnel::release")
}

// B6: only a status that lists the hub's own lease as v2 lets the front door read
// PROXY headers; every other outcome clears it.
func TestTunnelProxyProtocolFlag(t *testing.T) {
	h, f := tunnelHub(t, false)
	ctx := context.Background()
	check := func(step string, want bool) {
		t.Helper()
		h.reconcileTunnel(ctx)
		got, shown := h.tunnelProxy.Load(), ""
		if s := h.Status().Tunnel; s != nil {
			shown = s.ProxyProtocol
		}
		if got != want || (shown == "v2") != want {
			t.Fatalf("%s: flag %v, status %q, want %v", step, got, shown, want)
		}
	}
	check("lease without the field", false)
	f.proxy = "v2"
	check("lease v2 while starting", true)
	f.proxy = "v1"
	check("lease v1", false)
	f.proxy, f.dropLease = "v2", true
	check("lease not listed", false)
	f.dropLease = false
	f.state, f.url = "ready", "https://r0ute234abcd5678.abc234.opentunnel.xyz"
	check("lease acquired again", true)

	f.statusErr = iii.ErrTimeout
	check("status failed", false)
	if s := h.Status(); s.PublicURL != f.url {
		t.Fatalf("status failure dropped the URL: %+v", s)
	}
	f.statusErr = nil
	check("status back", true)

	h.tunnel.expires = time.Now().Add(time.Minute)
	f.acquireErr = iii.ErrTimeout
	check("renewal failed", false)
	f.acquireErr = nil
	check("renewed", true)

	var during bool
	f.onStatus = func() { during = h.tunnelProxy.Load() }
	h.tunnel.expires = time.Now().Add(4 * time.Minute)
	check("renewal keeping the lease id", true)
	if !during {
		t.Fatal("a renewal with the same lease id cleared the flag during the status call")
	}
	f.lease = "lease-new"
	h.tunnel.expires = time.Now().Add(4 * time.Minute)
	check("renewal with a new lease id", true)
	if during {
		t.Fatal("a new lease id kept the flag during the status call")
	}
	f.onStatus = nil

	h.config.PublicURL = "https://hub.example"
	check("public_url set", false)
	h.config.PublicURL = ""
	check("tunnel wanted again", true)
	h.config.Tunnel.Enabled = false
	check("tunnel disabled", false)
	h.config.Tunnel.Enabled = true
	check("tunnel enabled again", true)

	// The old target's tunnel stays up in an unknown mode until its release succeeds.
	f.releaseErr, h.config.Tunnel.TunnelID = iii.ErrTimeout, "mesh-2"
	check("tunnel_id changed, release failed", false)
	if h.tunnel.tunnelID != "mesh" {
		t.Fatalf("failed release dropped the old lease: %+v", h.tunnel)
	}
	f.releaseErr = nil
	check("old target released", true)

	h.tunnelClient = nil
	check("no client", false)
	h.tunnelClient = f
	check("client back", true)
	h.releaseTunnel()
	if h.tunnelProxy.Load() {
		t.Fatal("releaseTunnel kept the flag")
	}
}

// B8: the provider reported by a good status decides; a failed call keeps it.
func TestTunnelRawFollowsProvider(t *testing.T) {
	h, f := tunnelHub(t, false)
	ctx := context.Background()
	check := func(step string, want bool) {
		t.Helper()
		h.reconcileTunnel(ctx)
		if got := h.tunnelRaw.Load(); got != want {
			t.Fatalf("%s: tunnelRaw %v, want %v", step, got, want)
		}
	}
	check("no provider", false)
	f.provider = "opentunnel"
	check("opentunnel", true)
	f.statusErr = iii.ErrTimeout
	check("status failed", true)
	f.statusErr, f.provider = nil, ""
	check("provider absent again", false)
	f.provider = "opentunnel"
	check("opentunnel again", true)
	f.releaseErr = iii.ErrTimeout
	h.config.PublicURL = "https://hub.example"
	check("release failed, lease kept", true)
	f.releaseErr = nil
	check("released", false)
}
