package meshnet

import (
	"crypto/rand"
	"errors"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"net/netip"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/iii-hq/workers/mesh/internal/coordinator"
	"github.com/iii-hq/workers/mesh/internal/relay"
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

// newTestControl serves a coordinator and DERP relay behind a front door and
// returns the public URL nodes join through.
func newTestControl(t *testing.T) (*coordinator.Coordinator, string) {
	t.Helper()
	c, err := coordinator.New(t.TempDir(), func(secret, addr string) (string, error) { return "", nil })
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { c.Close() })
	admission := httptest.NewServer(relay.AdmitHandler(c.Known))
	t.Cleanup(admission.Close)
	derp := relay.NewWithKey(c.RelayKey(), admission.URL)
	t.Cleanup(func() { derp.Close() })
	mux := http.NewServeMux()
	c.Handle(mux)
	relay.Handle(mux, derp)
	front := httptest.NewServer(http.StripPrefix("/mesh", mux))
	t.Cleanup(front.Close)
	return c, front.URL + "/mesh/"
}

func TestNodeBackendPersistsIdentityAndHandlesRemoval(t *testing.T) {
	c, publicURL := newTestControl(t)
	hubShim := newTestShim(t, publicURL, c.Public())
	// The hub node must outlive the test: closing it mid-test drops the close the forwarder waits for.
	gateway, err := c.Listen(t.Context(), hubShim.URL(), 50221)
	if err != nil {
		t.Fatal(err)
	}
	dir := t.TempDir()
	c.SetGatewayPort(50221)
	c.SetNameTaken(func(name string) bool { return name == "laptop" })
	var first netip.Addr
	for i := 0; i < 2; i++ {
		s := newTestShim(t, publicURL, c.Public())
		m, err := startMesh(dir, s, rand.Text(), "laptop")
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() { m.Close() })
		waitFor(t, "node map", func() bool { return m.Self().IsValid() })
		if name, ok := m.SelfName(); !ok || name != "laptop-2" {
			t.Fatalf("assigned name %q, %v", name, ok)
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
		forward, err := m.Forward()
		if err != nil {
			t.Fatal(err)
		}
		forward = strings.TrimPrefix(forward, "ws://")
		local, err := net.Dial("tcp", forward)
		if err != nil {
			t.Fatal(err)
		}
		remote, err := gateway[0].Accept()
		if err != nil {
			t.Fatal(err)
		}
		local.SetReadDeadline(time.Now().Add(10 * time.Second))
		remote.SetReadDeadline(time.Now().Add(10 * time.Second))
		if got := netip.MustParseAddrPort(remote.RemoteAddr().String()).Addr(); got != m.Self() {
			t.Fatalf("gateway saw %s, want the client's tailnet address %s", got, m.Self())
		}
		buf := make([]byte, 2)
		if _, err := local.Write([]byte("up")); err != nil {
			t.Fatal(err)
		}
		if _, err := io.ReadFull(remote, buf); err != nil || string(buf) != "up" {
			t.Fatalf("hub read %q: %v", buf, err)
		}
		if _, err := remote.Write([]byte("dn")); err != nil {
			t.Fatal(err)
		}
		if _, err := io.ReadFull(local, buf); err != nil || string(buf) != "dn" {
			t.Fatalf("client read %q: %v", buf, err)
		}
		remote.Close()
		if _, err := local.Read(buf); err != io.EOF {
			t.Fatalf("hub close not forwarded: %v", err)
		}
		local.Close()
		for len(m.GatewayChanges()) > 0 {
			<-m.GatewayChanges()
		}
		c.SetGatewayPort(50222)
		select {
		case <-m.GatewayChanges():
		case <-time.After(10 * time.Second):
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
		waitFor(t, "forwarder closed with the node", func() bool {
			c, err := net.Dial("tcp", forward)
			if err == nil {
				c.Close()
			}
			return err != nil
		})
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

// The backend retries a refused join secret forever; Errors lets a caller stop it.
func TestRefusedRegistrationReachesErrors(t *testing.T) {
	c, err := coordinator.New(t.TempDir(), func(secret, addr string) (string, error) { return "", errors.New("token revoked") })
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	mux := http.NewServeMux()
	c.Handle(mux)
	front := httptest.NewServer(mux)
	defer front.Close()
	m, err := UpUserspace(t.TempDir(), front.URL, rand.Text(), "laptop", c.Public())
	if err != nil {
		t.Fatal(err)
	}
	defer m.Close()
	select {
	case msg := <-m.Errors():
		if !strings.Contains(msg, "token revoked") {
			t.Fatalf("error %q", msg)
		}
	case <-time.After(10 * time.Second):
		t.Fatal("refusal not reported")
	}
	if _, ok := m.SelfName(); ok {
		t.Fatal("refused node is named")
	}
}

func TestUpUserspaceRejectsInvalidName(t *testing.T) {
	if _, err := UpUserspace(t.TempDir(), "https://example.invalid", rand.Text(), "bad.name", key.NewMachine().Public()); err == nil {
		t.Fatal("userspace accepted invalid node name")
	}
}

// Security: netstack would otherwise forward unclaimed inbound flows to the
// client host's services. TestHubCannotReachClientHostLoopback in coordinator
// proves the refusal end to end.
func TestUserspaceNodeDeniesInboundFlows(t *testing.T) {
	c, publicURL := newTestControl(t)
	client, err := UpUserspace(t.TempDir(), publicURL, rand.Text(), "laptop", c.Public())
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	waitFor(t, "client address", func() bool { return client.Self().IsValid() })
	dst := netip.AddrPortFrom(client.Self(), 80)
	if h, ok := client.stack.GetTCPHandlerForFlow(netip.AddrPort{}, dst); h != nil || !ok {
		t.Fatal("inbound TCP is not denied")
	}
	if h, ok := client.stack.GetUDPHandlerForFlow(netip.AddrPort{}, dst); h != nil || !ok {
		t.Fatal("inbound UDP is not denied")
	}
	if err := client.Close(); err != nil {
		t.Fatal(err)
	}
}

// A hub without the endpoint, the hub's refusal and a failure to reach it are told apart.
func TestRenameAnswer(t *testing.T) {
	for _, status := range []int{http.StatusNotFound, http.StatusMethodNotAllowed} {
		if _, err := renameAnswer(status, strings.NewReader("404 page not found")); !errors.Is(err, ErrRenameUnsupported) {
			t.Fatalf("%d: %v", status, err)
		}
	}
	var refused *RenameRefused
	if name, err := renameAnswer(http.StatusOK, strings.NewReader(`{"name":"","error":"node keys refused"}`)); !errors.As(err, &refused) || refused.Reason != "node keys refused" || name != "" {
		t.Fatalf("refusal: %q %v", name, err)
	}
	if _, err := renameAnswer(http.StatusInternalServerError, strings.NewReader("boom")); err == nil || errors.As(err, &refused) || errors.Is(err, ErrRenameUnsupported) {
		t.Fatalf("server error: %v", err)
	}
	if _, err := renameAnswer(http.StatusOK, strings.NewReader("not json")); err == nil || errors.As(err, &refused) {
		t.Fatalf("garbled answer: %v", err)
	}
	if name, err := renameAnswer(http.StatusOK, strings.NewReader(`{"name":"work-2"}`)); err != nil || name != "work-2" {
		t.Fatalf("answer: %q %v", name, err)
	}
}
