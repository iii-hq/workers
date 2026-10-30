package meshnet

import (
	"context"
	"crypto/rand"
	"net"
	"net/http"
	"net/http/httptest"
	"net/netip"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/iii-hq/workers/mesh/internal/coordinator"
	"github.com/iii-hq/workers/mesh/internal/relay"
	"tailscale.com/ipn"
	"tailscale.com/types/key"
)

func waitFor(t *testing.T, what string, f func() bool) {
	t.Helper()
	timer := time.NewTimer(10 * time.Second)
	defer timer.Stop()
	ticker := time.NewTicker(10 * time.Millisecond)
	defer ticker.Stop()
	for {
		if f() {
			return
		}
		select {
		case <-timer.C:
			t.Fatal("timed out waiting for " + what)
		case <-ticker.C:
		}
	}
}

func TestNodeBackendPersistsIdentityUsesPortAndHandlesRemoval(t *testing.T) {
	c, err := coordinator.New(t.TempDir(), func(secret, addr string) (string, error) { return "", nil })
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	admission := httptest.NewServer(relay.AdmitHandler(c.Known))
	defer admission.Close()
	derp := relay.NewWithKey(c.RelayKey(), admission.URL)
	defer derp.Close()
	mux := http.NewServeMux()
	c.Handle(mux)
	relay.Handle(mux, derp)
	front := httptest.NewServer(http.StripPrefix("/mesh", mux))
	defer front.Close()
	hubShim := newTestShim(t, front.URL+"/mesh/", c.Public())
	ctx, cancel := context.WithTimeout(t.Context(), 15*time.Second)
	defer cancel()
	if _, err := c.Listen(ctx, hubShim.URL()); err != nil {
		t.Fatal(err)
	}
	dir := t.TempDir()
	hosts := filepath.Join(t.TempDir(), "hosts")
	if err := os.WriteFile(hosts, []byte("127.0.0.1 localhost\n"), 0644); err != nil {
		t.Fatal(err)
	}
	portProbe, err := net.ListenPacket("udp4", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	c.SetGatewayPort(50221)
	c.SetNameTaken(func(name string) bool { return name == "laptop" })
	port := uint16(portProbe.LocalAddr().(*net.UDPAddr).Port)
	portProbe.Close()
	var first netip.Addr
	for i := 0; i < 2; i++ {
		s := newTestShim(t, front.URL+"/mesh/", c.Public())
		m, err := startMesh(dir, s, rand.Text(), "laptop", port, nil, nil, hosts)
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() { m.Close() })
		waitFor(t, "node map", func() bool { return m.Self().IsValid() })
		if name, ok := m.SelfName(); !ok || name != "laptop-2" {
			t.Fatalf("assigned name %q, %v", name, ok)
		}
		if got := m.sys.MagicSock.Get().LocalPort(); got != port {
			t.Fatalf("UDP port=%d, want %d", got, port)
		}
		if i == 0 {
			first = m.Self()
		} else if m.Self() != first {
			t.Fatalf("node identity changed from %s to %s", first, m.Self())
		}
		if m.lb.TryFlushLogs() {
			t.Fatal("backend started a log uploader")
		}
		if m.lb.Prefs().CorpDNS() {
			t.Fatal("node must not configure host DNS")
		}
		waitFor(t, "hub peer", func() bool { _, ok := m.Peer("hub"); return ok })
		waitFor(t, "authenticated gateway port", func() bool { p, ok := m.GatewayPort(); return ok && p == 50221 })
		for len(m.GatewayChanges()) > 0 {
			<-m.GatewayChanges()
		}
		c.SetGatewayPort(50222)
		select {
		case <-m.GatewayChanges():
		case <-ctx.Done():
			t.Fatal("gateway update missing")
		}
		waitFor(t, "updated gateway port", func() bool { p, ok := m.GatewayPort(); return ok && p == 50222 })
		c.SetGatewayPort(50221)
		if _, ok := m.Peer("absent"); ok {
			t.Fatal("unknown peer found")
		}
		if len(c.Nodes()) != 2 {
			t.Fatalf("restart registered new node: %+v", c.Nodes())
		}
		if i == 1 {
			if err := c.Remove("laptop-2"); err != nil {
				t.Fatal(err)
			}
			select {
			case <-m.Removed():
			case <-time.After(10 * time.Second):
				t.Fatal("removal notification missing")
			}
			if _, err := os.Stat(filepath.Join(dir, Removed)); err != nil {
				t.Fatal(err)
			}
		}
		if err := m.Close(); err != nil {
			t.Fatal(err)
		}
		if _, ok := m.GatewayPort(); ok {
			t.Fatal("closed node still advertises gateway")
		}
	}
	b, err := os.ReadFile(hosts)
	if err != nil || string(b) != "127.0.0.1 localhost\n" {
		t.Fatalf("hosts cleanup %q: %v", b, err)
	}
	files, err := os.ReadDir(dir)
	if err != nil {
		t.Fatal(err)
	}
	for _, f := range files {
		if f.Name() != "state" && f.Name() != Removed && f.Name() != "profile-data" {
			t.Fatalf("unexpected state/log artifact %s", f.Name())
		}
	}
}

func TestNodeBackendLogout(t *testing.T) {
	c, err := coordinator.New(t.TempDir(), func(secret, addr string) (string, error) { return "", nil })
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	mux := http.NewServeMux()
	c.Handle(mux)
	front := httptest.NewServer(mux)
	defer front.Close()
	hosts := filepath.Join(t.TempDir(), "hosts")
	os.WriteFile(hosts, nil, 0644)
	m, err := startMesh(t.TempDir(), newTestShim(t, front.URL, c.Public()), rand.Text(), "laptop", 0, nil, nil, hosts)
	if err != nil {
		t.Fatal(err)
	}
	defer m.Close()
	waitFor(t, "running", func() bool { return m.lb.State() == ipn.Running })
	if err := m.Logout(); err != nil {
		t.Fatal(err)
	}
	waitFor(t, "logout", func() bool { return len(c.Nodes()) == 0 })
}

func TestUpRejectsInvalidInputBeforeCreatingTUN(t *testing.T) {
	_, err := Up(t.TempDir(), "https://example.invalid", rand.Text(), "bad.name", key.NewMachine().Public())
	if err == nil {
		t.Fatal("accepted invalid node name")
	}
}
