package main

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

type fakeTunnelRPC struct {
	mu               sync.Mutex
	now              time.Time
	status           string
	url              string
	cloudflared      bool
	listener         any
	leases           []tunnelLease
	calls            map[string]int
	released         []string
	loseAcquireReply bool
	failStatus       bool
	failRelease      bool
}

func newTunnelFixture(t *testing.T) (*DevelopmentTunnel, *fakeTunnelRPC) {
	t.Helper()
	cfg := DefaultConfig()
	cfg.DataDir = t.TempDir()
	cfg.Cloud.Enabled = true
	cfg.Cloud.AppSecret = "secret-sentinel"
	cfg.Cloud.VerifyToken = "verify-sentinel"
	fake := &fakeTunnelRPC{now: time.Now().UTC(), status: "ready", url: "https://first-valid.trycloudflare.com", cloudflared: true, listener: map[string]any{"host": "127.0.0.1", "port": 3129}, calls: map[string]int{}}
	service := newDevelopmentTunnel(context.Background(), func(context.Context) (Config, error) { return cfg, nil }, fake.call)
	service.now = func() time.Time { fake.mu.Lock(); defer fake.mu.Unlock(); return fake.now }
	return service, fake
}
func (f *fakeTunnelRPC) call(_ context.Context, id string, input any) (json.RawMessage, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.calls[id]++
	params := input.(map[string]any)
	switch id {
	case "http::webhook-listener::status":
		return json.Marshal(map[string]any{"applied": f.listener})
	case "quick-tunnel::status":
		if f.failStatus {
			return nil, errors.New("transport secret-sentinel")
		}
		return json.Marshal(map[string]any{"tunnel_id": "webhooks", "status": f.status, "public_url": f.url, "leases": f.leases, "prerequisites": map[string]any{"cloudflared": map[string]bool{"found": f.cloudflared}}})
	case "quick-tunnel::acquire":
		consumer := params["consumer_id"].(string)
		expiry, err := time.Parse(time.RFC3339Nano, params["expires_at"].(string))
		if err != nil {
			return nil, err
		}
		found := false
		for i := range f.leases {
			if f.leases[i].ConsumerID == consumer {
				f.leases[i].ExpiresAt = expiry
				found = true
			}
		}
		if !found {
			f.leases = append(f.leases, tunnelLease{"whatsapp-lease", consumer, "webhooks", expiry})
		}
		if f.loseAcquireReply {
			return nil, errors.New("reply lost")
		}
		return json.Marshal(map[string]any{"lease_id": "whatsapp-lease", "tunnel_id": "webhooks", "status": f.status, "public_url": f.url})
	case "quick-tunnel::release":
		if f.failRelease {
			return nil, errors.New("release failed secret-sentinel")
		}
		id := params["lease_id"].(string)
		f.released = append(f.released, id)
		active := f.leases[:0]
		for _, lease := range f.leases {
			if lease.LeaseID != id {
				active = append(active, lease)
			}
		}
		f.leases = active
		return json.RawMessage(`{"released":true}`), nil
	default:
		return nil, errors.New("unexpected function " + id)
	}
}
func tunnelResult(t *testing.T, status DevelopmentTunnelStatus, err error, want string) DevelopmentTunnelStatus {
	t.Helper()
	if err != nil {
		t.Fatal(err)
	}
	if status.Status != want {
		t.Fatalf("status=%+v, want %s", status, want)
	}
	encoded, _ := json.Marshal(status)
	if strings.Contains(string(encoded), "secret-sentinel") {
		t.Fatal("secret leaked")
	}
	if want != "ready" && (status.PublicURL != nil || status.WebhookURL != nil) {
		t.Fatal("unready tunnel exposed URL")
	}
	return status
}
func TestDevelopmentTunnelDisabledNeedsNoDependencies(t *testing.T) {
	service, fake := newTunnelFixture(t)
	got, err := service.Status(context.Background())
	tunnelResult(t, got, err, "stopped")
	if len(fake.calls) != 0 {
		t.Fatalf("disabled status invoked dependencies: %v", fake.calls)
	}
}
func TestDevelopmentTunnelOwnLeaseAndConcurrentStart(t *testing.T) {
	service, fake := newTunnelFixture(t)
	fake.leases = []tunnelLease{{"github-lease", "github", "webhooks", fake.now.Add(time.Hour)}}
	var group sync.WaitGroup
	for range 10 {
		group.Add(1)
		go func() {
			defer group.Done()
			got, err := service.Start(context.Background())
			tunnelResult(t, got, err, "ready")
		}()
	}
	group.Wait()
	if fake.calls["quick-tunnel::acquire"] != 1 {
		t.Fatalf("duplicate leases: %v", fake.calls)
	}
	if len(fake.leases) != 2 {
		t.Fatalf("leases=%v", fake.leases)
	}
	got, err := service.Status(context.Background())
	status := tunnelResult(t, got, err, "ready")
	if *status.WebhookURL != fake.url+cloudWebhookPath() {
		t.Fatalf("callback=%s", *status.WebhookURL)
	}
	got, err = service.Stop(context.Background())
	tunnelResult(t, got, err, "stopped")
	got, err = service.Stop(context.Background())
	tunnelResult(t, got, err, "stopped")
	if len(fake.released) != 1 || fake.released[0] != "whatsapp-lease" || len(fake.leases) != 1 || fake.leases[0].ConsumerID != "github" {
		t.Fatalf("released foreign lease: %v %v", fake.released, fake.leases)
	}
	if got.Enabled {
		t.Fatal("stop retained enabled intent")
	}
}
func TestDevelopmentTunnelTracksAuthoritativeLifecycleAndExpiry(t *testing.T) {
	service, fake := newTunnelFixture(t)
	got, err := service.Start(context.Background())
	tunnelResult(t, got, err, "ready")
	for _, state := range []string{"reconnecting", "starting", "failed", "stopped"} {
		fake.status = state
		got, err = service.Status(context.Background())
		tunnelResult(t, got, err, state)
	}
	fake.status = "ready"
	fake.url = "https://rotated.trycloudflare.com"
	service.Notify() // A notification never supplies or restores a cached URL.
	got, err = service.Status(context.Background())
	tunnelResult(t, got, err, "ready")
	if *got.PublicURL != fake.url {
		t.Fatal("retained old generation URL")
	}
	fake.now = fake.now.Add(developmentTunnelTTL + time.Second)
	got, err = service.Status(context.Background())
	tunnelResult(t, got, err, "starting")
	got, err = service.Start(context.Background())
	tunnelResult(t, got, err, "ready")
	if fake.calls["quick-tunnel::acquire"] != 2 {
		t.Fatal("expired lease was not renewed")
	}
	fake.failStatus = true
	got, err = service.Status(context.Background())
	tunnelResult(t, got, err, "unavailable")
}
func TestDevelopmentTunnelLostAcquireReplyAndRestartRecovery(t *testing.T) {
	service, fake := newTunnelFixture(t)
	fake.loseAcquireReply = true
	got, err := service.Start(context.Background())
	tunnelResult(t, got, err, "ready")
	recovered := newDevelopmentTunnel(context.Background(), service.config, fake.call)
	recovered.now = service.now
	if recovered.state.PublicURL != nil || recovered.state.Status != "starting" {
		t.Fatal("restart trusted cached readiness")
	}
	got, err = recovered.Start(context.Background())
	tunnelResult(t, got, err, "ready")
	if fake.calls["quick-tunnel::acquire"] != 1 {
		t.Fatal("restart acquired duplicate lease")
	}
	if err = recovered.Close(context.Background()); err != nil {
		t.Fatal(err)
	}
	if len(fake.leases) != 0 {
		t.Fatal("shutdown left lease")
	}
	var persisted tunnelIntent
	raw, err := os.ReadFile(service.path)
	if err != nil || json.Unmarshal(raw, &persisted) != nil || !persisted.Enabled || persisted.LeaseID != "" {
		t.Fatalf("shutdown lost requested intent: %s %v", raw, err)
	}
	resumed := newDevelopmentTunnel(context.Background(), service.config, fake.call)
	resumed.now = service.now
	resumed.Run(context.Background())
	deadline := time.After(2 * time.Second)
	for {
		fake.mu.Lock()
		acquired := fake.calls["quick-tunnel::acquire"] == 2
		fake.mu.Unlock()
		if acquired {
			break
		}
		select {
		case <-deadline:
			t.Fatal("enabled intent did not resume")
		case <-time.After(time.Millisecond):
		}
	}
	if err = resumed.Close(context.Background()); err != nil {
		t.Fatal(err)
	}
}
func TestDevelopmentTunnelStopRecoversUnknownLeaseAndRetriesAfterRestart(t *testing.T) {
	service, fake := newTunnelFixture(t)
	service.intent.Enabled = true
	fake.leases = []tunnelLease{{"unknown-reply-lease", service.intent.ConsumerID, "webhooks", fake.now.Add(developmentTunnelTTL)}}
	fake.failRelease = true
	got, err := service.Stop(context.Background())
	tunnelResult(t, got, err, "unavailable")
	if got.Enabled {
		t.Fatal("failed release retained enabled intent")
	}
	restarted := newDevelopmentTunnel(context.Background(), service.config, fake.call)
	restarted.now = service.now
	fake.failRelease = false
	got, err = restarted.Status(context.Background())
	tunnelResult(t, got, err, "stopped")
	if len(fake.leases) != 0 || len(fake.released) != 1 || fake.released[0] != "unknown-reply-lease" {
		t.Fatal("failed stop was not recovered")
	}
}
func TestDevelopmentTunnelPreflightAndURLValidation(t *testing.T) {
	for _, test := range []struct {
		name   string
		update func(*DevelopmentTunnel, *fakeTunnelRPC)
	}{
		{"cloud disabled", func(s *DevelopmentTunnel, _ *fakeTunnelRPC) {
			c, _ := s.config(context.Background())
			c.Cloud.Enabled = false
			s.config = func(context.Context) (Config, error) { return c, nil }
		}},
		{"missing verify token", func(s *DevelopmentTunnel, _ *fakeTunnelRPC) {
			c, _ := s.config(context.Background())
			c.Cloud.VerifyToken = ""
			s.config = func(context.Context) (Config, error) { return c, nil }
		}},
		{"listener disabled", func(_ *DevelopmentTunnel, f *fakeTunnelRPC) { f.listener = nil }},
		{"public listener", func(_ *DevelopmentTunnel, f *fakeTunnelRPC) {
			f.listener = map[string]any{"host": "0.0.0.0", "port": 3112}
		}},
		{"missing binary", func(_ *DevelopmentTunnel, f *fakeTunnelRPC) { f.cloudflared = false }},
		{"worker unavailable", func(_ *DevelopmentTunnel, f *fakeTunnelRPC) { f.failStatus = true }},
	} {
		t.Run(test.name, func(t *testing.T) {
			service, fake := newTunnelFixture(t)
			test.update(service, fake)
			got, err := service.Start(context.Background())
			tunnelResult(t, got, err, "unavailable")
			if fake.calls["quick-tunnel::acquire"] != 0 {
				t.Fatal("acquired before prerequisites")
			}
		})
	}
	for _, url := range []string{"http://valid.trycloudflare.com", "https://evil.com", "https://sub.sub.trycloudflare.com", "https://good.trycloudflare.com/path", "https://good.trycloudflare.com:443", "https://user@good.trycloudflare.com", "https://good.trycloudflare.com?token=x", "https://-bad.trycloudflare.com", "https://good.trycloudflare.com/"} {
		t.Run(url, func(t *testing.T) {
			service, fake := newTunnelFixture(t)
			fake.url = url
			got, err := service.Start(context.Background())
			tunnelResult(t, got, err, "failed")
		})
	}
}
func TestDevelopmentTunnelPersistenceIsPrivateAndCannotReleaseForeignLease(t *testing.T) {
	service, fake := newTunnelFixture(t)
	fake.leases = []tunnelLease{{"github-lease", "github", "webhooks", fake.now.Add(time.Hour)}}
	service.intent.LeaseID = "github-lease"
	service.intent.ReleasePending = true
	if err := service.save(); err != nil {
		t.Fatal(err)
	}
	info, err := os.Stat(service.path)
	if err != nil || info.Mode().Perm() != 0600 {
		t.Fatalf("unsafe mode: %v %v", info, err)
	}
	got, err := service.Stop(context.Background())
	tunnelResult(t, got, err, "stopped")
	if len(fake.released) != 0 || len(fake.leases) != 1 {
		t.Fatal("persisted ID released another consumer")
	}
	foreign := filepath.Join(t.TempDir(), "foreign.json")
	if err = os.WriteFile(foreign, []byte(`{}`), 0644); err != nil {
		t.Fatal(err)
	}
	if err = os.Remove(service.path); err != nil {
		t.Fatal(err)
	}
	if err = os.Symlink(foreign, service.path); err != nil {
		t.Fatal(err)
	}
	unsafe := newDevelopmentTunnel(context.Background(), service.config, fake.call)
	got, err = unsafe.Start(context.Background())
	tunnelResult(t, got, err, "unavailable")
	if fake.calls["quick-tunnel::acquire"] != 0 {
		t.Fatal("unsafe persistence acquired a tunnel")
	}
}
func TestDevelopmentTunnelConsumerIdentityIsBoundedAndNamespaced(t *testing.T) {
	id := tunnelConsumerID(strings.Repeat("namespace", 1000), strings.Repeat("config", 1000))
	if len(id) > 128 || id != tunnelConsumerID(strings.Repeat("namespace", 1000), strings.Repeat("config", 1000)) {
		t.Fatal("unbounded or unstable consumer identity")
	}
	if tunnelConsumerID("one", "same") == tunnelConsumerID("two", "same") || tunnelConsumerID("ab", "c") == tunnelConsumerID("a", "bc") {
		t.Fatal("consumer identity collision")
	}
}

func TestDevelopmentTunnelRenewalWakesWithoutChangingConsumer(t *testing.T) {
	service, fake := newTunnelFixture(t)
	got, err := service.Start(context.Background())
	tunnelResult(t, got, err, "ready")
	fake.mu.Lock()
	fake.now = fake.now.Add(3 * time.Minute)
	fake.mu.Unlock()
	service.Run(context.Background())
	deadline := time.After(2 * time.Second)
	for {
		fake.mu.Lock()
		renewed := fake.calls["quick-tunnel::acquire"] == 2
		fake.mu.Unlock()
		if renewed {
			break
		}
		select {
		case <-deadline:
			t.Fatal("lease was not renewed before expiry")
		case <-time.After(time.Millisecond):
		}
	}
	fake.mu.Lock()
	if len(fake.leases) != 1 || fake.leases[0].ConsumerID != service.intent.ConsumerID || fake.leases[0].ExpiresAt.Sub(fake.now) != developmentTunnelTTL {
		t.Errorf("renewal changed identity or TTL: %v", fake.leases)
	}
	fake.mu.Unlock()
	if err = service.Close(context.Background()); err != nil {
		t.Fatal(err)
	}
}
func TestDevelopmentTunnelReportsShutdownReleaseFailure(t *testing.T) {
	service, fake := newTunnelFixture(t)
	got, err := service.Start(context.Background())
	tunnelResult(t, got, err, "ready")
	fake.failRelease = true
	if err = service.Close(context.Background()); err == nil {
		t.Fatal("shutdown hid failed release")
	}
	if service.state.WebhookURL != nil {
		t.Fatal("shutdown retained URL")
	}
	var intent tunnelIntent
	data, _ := os.ReadFile(service.path)
	if json.Unmarshal(data, &intent) != nil || !intent.Enabled || intent.LeaseID == "" {
		t.Fatal("shutdown discarded recovery intent")
	}
}
