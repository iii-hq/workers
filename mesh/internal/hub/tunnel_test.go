package hub

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"reflect"
	"strings"
	"sync"
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/coordinator"
	"github.com/iii-hq/workers/mesh/internal/join"
)

// tunnelRPC fakes quick-tunnel's acquire/status/release contract.
type tunnelRPC struct {
	mu                                sync.Mutex // the hub's loop calls while a test changes the answers
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
	f.mu.Lock()
	defer f.mu.Unlock()
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
func (f *tunnelRPC) take() []string {
	f.mu.Lock()
	defer f.mu.Unlock()
	c := f.calls
	f.calls = nil
	return c
}
func (f *tunnelRPC) set(change func()) { f.mu.Lock(); defer f.mu.Unlock(); change() }

// count is how many calls of id the fake took; "" counts every call.
func (f *tunnelRPC) count(id string) (n int) {
	f.mu.Lock()
	defer f.mu.Unlock()
	for _, c := range f.calls {
		if id == "" || c == id {
			n++
		}
	}
	return n
}

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

// shortLease lowers the lease times for a test that waits for them.
func shortLease(t *testing.T, lease, renew time.Duration) {
	oldLease, oldRenew := tunnelLease, tunnelRenew
	tunnelLease, tunnelRenew = lease, renew
	t.Cleanup(func() { tunnelLease, tunnelRenew = oldLease, oldRenew })
}

// tunnelLoop is a tunnel hub with its loop running; first is the retry's first wait.
func tunnelLoop(t *testing.T, first time.Duration) (*Hub, *tunnelRPC) {
	t.Helper()
	old := retryMin
	retryMin = first
	h, f := tunnelHub(t, false)
	h.proxyKick, h.Sessions = make(chan struct{}, 1), &Sessions{}
	done := make(chan struct{})
	go func() { defer close(done); h.serve(t.Context()) }()
	t.Cleanup(func() {
		<-done // the test's context ended before the cleanups
		h.tunnelRetry.reset()
		if h.tunnelDue != nil {
			h.tunnelDue.Stop()
		}
		retryMin = old
	})
	return h, f
}

func TestTunnelLeaseIsRenewedByItsDeadline(t *testing.T) {
	shortLease(t, 600*time.Millisecond, 400*time.Millisecond)
	h, f := tunnelLoop(t, time.Hour)
	h.kickTunnel() // the only event
	within(t, "the lease was not renewed when it was due", func() bool { return f.count("quick-tunnel::acquire") >= 2 })
	var ends []time.Time // of each lease asked for
	f.set(func() {
		for i, id := range f.calls {
			if end, err := time.Parse(time.RFC3339Nano, fmt.Sprint(f.payloads[i]["expires_at"])); id == "quick-tunnel::acquire" && err == nil {
				ends = append(ends, end)
			}
		}
	})
	if late := ends[1].Sub(ends[0]); late >= tunnelLease {
		t.Fatalf("renewed %s after the acquire: the %s lease had ended", late, tunnelLease)
	}
}

func TestTunnelRetriesOnlyWhatNoEventEnds(t *testing.T) {
	const acquire, status, url = "quick-tunnel::acquire", "quick-tunnel::status", "https://abc234.opentunnel.xyz"
	t.Run("an acquire that times out", func(t *testing.T) {
		h, f := tunnelLoop(t, 5*time.Millisecond)
		f.set(func() { f.acquireErr, f.state, f.url = iii.ErrTimeout, "ready", url })
		h.kickTunnel() // the only event
		within(t, "a timed out acquire was not tried again", func() bool { return f.count(acquire) >= 3 })
		if s := h.Status(); s.PublicURL != "" || s.Tunnel.State != "unavailable" {
			t.Fatalf("status while acquire fails: %+v %+v", s, s.Tunnel)
		}
		f.set(func() { f.acquireErr = nil })
		within(t, "the retry did not publish the URL", func() bool { return h.Status().PublicURL == url })
	})
	t.Run("a status that times out", func(t *testing.T) {
		h, f := tunnelLoop(t, 5*time.Millisecond)
		f.set(func() { f.statusErr = iii.ErrTimeout })
		h.kickTunnel()
		within(t, "a timed out status was not tried again", func() bool { return f.count(status) >= 3 })
	})
	t.Run("a lease quick-tunnel dropped", func(t *testing.T) {
		h, f := tunnelLoop(t, 5*time.Millisecond)
		f.set(func() { f.dropLease = true })
		h.kickTunnel()
		within(t, "a lease that status does not list was not acquired again", func() bool { return f.count(acquire) >= 2 })
	})
	for name, err := range map[string]error{
		"quick-tunnel is not installed":   &iii.InvocationError{Code: "function_not_found", Message: acquire},
		"quick-tunnel has no such target": errors.New("invalid consumer_id or unauthorized tunnel_id"),
	} {
		t.Run(name, func(t *testing.T) {
			h, f := tunnelLoop(t, 5*time.Millisecond)
			f.set(func() { f.acquireErr = err })
			h.kickTunnel()
			within(t, "the event did not run a pass", func() bool { return f.count(acquire) > 0 })
			time.Sleep(200 * time.Millisecond)
			if n := f.count(acquire); n != 1 {
				t.Fatalf("%d acquires in 200 ms for a failure quick-tunnel starting ends, want 1", n)
			}
			h.functionsChanged() // what quick-tunnel starting causes
			within(t, "a function change did not run the tunnel pass", func() bool { return f.count(acquire) == 2 })
		})
	}
}

// A lease the hub no longer wants is released by the retry: nothing announces that quick-tunnel answers again.
func TestTunnelRetriesFailedRelease(t *testing.T) {
	const acquire, release = "quick-tunnel::acquire", "quick-tunnel::release"
	for name, change := range map[string]func(*Config){
		"the tunnel is turned off": func(c *Config) { c.Tunnel.Enabled = false },
		"the tunnel id changes":    func(c *Config) { c.Tunnel.TunnelID = "hub" },
	} {
		t.Run(name, func(t *testing.T) {
			h, f := tunnelLoop(t, 5*time.Millisecond)
			h.kickTunnel()
			within(t, "no lease", func() bool { return f.count("quick-tunnel::status") == 1 })
			f.set(func() { f.releaseErr = iii.ErrTimeout })
			h.mu.Lock()
			change(&h.config)
			h.mu.Unlock()
			h.kickTunnel() // the configuration change: the last event
			within(t, "the event did not run a pass", func() bool { return f.count(release) > 0 })
			time.Sleep(200 * time.Millisecond)
			if n := f.count(release); n > 8 { // 5, 10, 20, 40, 80 ms apart: 6 releases
				t.Fatalf("%d releases in 200 ms, want a doubling wait", n)
			}
			within(t, "the failed release was not tried again", func() bool { return f.count(release) >= 3 })
			if n := f.count(acquire); n != 1 {
				t.Fatalf("%d acquires while the old lease is held, want 1", n)
			}
			f.set(func() { f.releaseErr = nil })
			held := func() string { h.tunnelMu.Lock(); defer h.tunnelMu.Unlock(); return h.tunnel.tunnelID }
			within(t, "the retry did not release the lease", func() bool { return held() != "mesh" })
			if h.Config().Tunnel.Enabled { // the new target waited for the old lease's release
				within(t, "the new target was not acquired", func() bool { return held() == "hub" && f.count("quick-tunnel::status") == 2 })
			}
			n := f.count("")
			if time.Sleep(100 * time.Millisecond); f.count("") != n {
				t.Fatal("a call followed the release")
			}
		})
	}
}

// A pass that ends with nothing to retry leaves no retry waiting, and the next failure starts from the first wait.
func TestTunnelPassThatEndsStopsItsRetry(t *testing.T) {
	defer func(d time.Duration) { retryMin = d }(retryMin)
	retryMin = time.Minute
	h, f := tunnelHub(t, false)
	f.acquireErr = iii.ErrTimeout
	if h.reconcileTunnel(t.Context()); h.tunnelRetry.wait != time.Minute {
		t.Fatalf("wait %s after a failure no event ends, want a retry", h.tunnelRetry.wait)
	}
	f.acquireErr = nil
	h.reconcileTunnel(t.Context())
	defer h.tunnelDue.Stop()
	if h.tunnelRetry.wait != 0 || h.tunnelRetry.timer.Stop() {
		t.Fatal("the pass left its retry waiting")
	}
}

// A lease that ends while quick-tunnel is away has no event: its deadline stops the URL, with one pass and no spin.
func TestTunnelStopsPublishingWhenTheLeaseEnds(t *testing.T) {
	const acquire, status, url = "quick-tunnel::acquire", "quick-tunnel::status", "https://abc234.opentunnel.xyz"
	for _, tc := range []struct {
		name  string
		event bool
		want  []string
	}{
		{"an event finds quick-tunnel gone", true, []string{acquire, status, status, acquire}},
		{"the renewal finds quick-tunnel gone", false, []string{acquire, status, acquire, acquire}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			shortLease(t, time.Second, 400*time.Millisecond)
			h, f := tunnelLoop(t, time.Hour)
			f.set(func() { f.state, f.url = "ready", url })
			h.kickTunnel()
			within(t, "the tunnel did not turn ready", func() bool { return h.Status().PublicURL == url })
			missing := &iii.InvocationError{Code: "function_not_found"}
			f.set(func() { f.acquireErr, f.statusErr = missing, missing })
			if tc.event {
				h.kickTunnel() // the last event
			}
			within(t, "the failed call was not reported", func() bool { return h.Status().Tunnel.Error == tunnelNotResponding })
			if got := h.Status().PublicURL; got != url {
				t.Fatalf("public URL %q while the lease holds, want %q", got, url)
			}
			within(t, "the URL outlived its lease", func() bool { s := h.Status(); return s.PublicURL == "" && s.Tunnel.State == "unavailable" })
			time.Sleep(150 * time.Millisecond)
			wantCalls(t, f, tc.want...) // a deadline in the past would spin
		})
	}
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
