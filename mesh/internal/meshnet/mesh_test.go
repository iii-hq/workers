package meshnet

import (
	"context"
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

func TestNodeBackendPersistsIdentityUsesPortAndHandlesRemoval(t *testing.T) {
	c, publicURL := newTestControl(t)
	hubShim := newTestShim(t, publicURL, c.Public())
	// The hub node must outlive the test: closing it mid-test drops the close the forwarder waits for.
	gateway, err := c.Listen(t.Context(), hubShim.URL(), 50221)
	if err != nil {
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
		s := newTestShim(t, publicURL, c.Public())
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

func TestUpRejectsInvalidInputBeforeCreatingTUN(t *testing.T) {
	_, err := Up(t.TempDir(), "https://example.invalid", rand.Text(), "bad.name", key.NewMachine().Public())
	if err == nil {
		t.Fatal("accepted invalid node name")
	}
	if _, err := UpUserspace(t.TempDir(), "https://example.invalid", rand.Text(), "bad.name", key.NewMachine().Public()); err == nil {
		t.Fatal("userspace accepted invalid node name")
	}
}

// Security: a tailnet peer must never reach the client host's services, which
// netstack would otherwise forward unclaimed inbound flows to.
func TestUserspaceNodeRefusesInboundToHostLoopback(t *testing.T) {
	c, publicURL := newTestControl(t)
	c.Link([][2]string{{"laptop", "intruder"}}) // as the hub links drive peers
	service, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer service.Close()
	datagrams, err := net.ListenPacket("udp4", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer datagrams.Close()
	accepted := make(chan struct{}, 1)
	go func() {
		if conn, err := service.Accept(); err == nil {
			conn.Close()
			accepted <- struct{}{}
		}
	}()
	client, err := UpUserspace(t.TempDir(), publicURL, rand.Text(), "laptop", c.Public())
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	peer, err := UpUserspace(t.TempDir(), publicURL, rand.Text(), "intruder", c.Public())
	if err != nil {
		t.Fatal(err)
	}
	defer peer.Close()
	waitFor(t, "client address", func() bool { return client.Self().IsValid() })
	waitFor(t, "maps", func() bool {
		_, a := peer.Peer("laptop")
		_, b := client.Peer("intruder")
		return a && b
	})
	// Long enough for magicsock to fall back to DERP when a direct path drops data.
	dial := func(port uint16) (net.Conn, error) {
		ctx, cancel := context.WithTimeout(t.Context(), 30*time.Second)
		defer cancel()
		return peer.stack.DialContextTCP(ctx, netip.AddrPortFrom(client.Self(), port))
	}
	// A netstack listener proves the peer path, so the refusal below is not a timeout.
	open, err := client.stack.ListenTCP("tcp6", netip.AddrPortFrom(client.Self(), 7).String())
	if err != nil {
		t.Fatal(err)
	}
	defer open.Close()
	conn, err := dial(7)
	if err != nil {
		t.Fatalf("peer path: %v", err)
	}
	conn.Close()
	port := uint16(service.Addr().(*net.TCPAddr).Port)
	if conn, err := dial(port); err == nil {
		conn.Close()
		t.Fatal("tailnet peer reached a 127.0.0.1 service on the client")
	} else if !strings.Contains(err.Error(), "refused") {
		t.Fatalf("want connection refused, got %v", err)
	}
	// The mesh prefix is outside tsaddr.IsTailscaleIP, so undenied TCP would be
	// dialed to [client]:port through the host OS, which this host cannot show.
	if h, ok := client.stack.GetTCPHandlerForFlow(netip.AddrPort{}, netip.AddrPortFrom(client.Self(), port)); h != nil || !ok {
		t.Fatal("inbound TCP is not denied")
	}
	// Undenied UDP to a local address is forwarded to 127.0.0.1:port.
	udp, err := peer.stack.DialContextUDP(t.Context(), netip.AddrPortFrom(client.Self(), uint16(datagrams.LocalAddr().(*net.UDPAddr).Port)))
	if err != nil {
		t.Fatal(err)
	}
	defer udp.Close()
	if _, err := udp.Write([]byte("x")); err != nil {
		t.Fatal(err)
	}
	datagrams.SetReadDeadline(time.Now().Add(time.Second))
	if _, _, err := datagrams.ReadFrom(make([]byte, 1)); err == nil {
		t.Fatal("tailnet peer reached a 127.0.0.1 UDP service on the client")
	}
	select {
	case <-accepted:
		t.Fatal("loopback service accepted a tailnet peer")
	default:
	}
	// An empty hosts path: nothing to write or restore, so Close is clean.
	if err := client.Close(); err != nil {
		t.Fatal(err)
	}
}
