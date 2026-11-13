package coordinator

import (
	"bufio"
	"context"
	"crypto/rand"
	"hash/crc32"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"net/netip"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/iii-hq/workers/mesh/internal/meshnet"
	"github.com/iii-hq/workers/mesh/internal/relay"
	"tailscale.com/feature/buildfeatures"
	"tailscale.com/net/netmon"
	"tailscale.com/tailcfg"
	"tailscale.com/tstest"
	"tailscale.com/util/eventbus"
)

func TestUserspacePrunesRetiredListeners(t *testing.T) {
	u := &userspace{}
	for range 20 {
		raw, err := net.Listen("tcp", "127.0.0.1:0")
		if err != nil {
			t.Fatal(err)
		}
		u.mu.Lock()
		ln := u.ownListenerLocked(raw)
		u.mu.Unlock()
		if err := ln.Close(); err != nil {
			t.Fatal(err)
		}
		u.mu.Lock()
		retained := len(u.listeners)
		u.mu.Unlock()
		if retained != 0 {
			t.Fatalf("retained retired listeners: %d", retained)
		}
	}
	raw, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	u.mu.Lock()
	ln := u.ownListenerLocked(raw)
	u.mu.Unlock()
	var wg sync.WaitGroup
	wg.Go(func() { u.close() })
	wg.Go(func() { ln.Close() })
	wg.Wait()
	if c, err := net.Dial("tcp", ln.Addr().String()); err == nil {
		c.Close()
		t.Fatal("shutdown left listener open")
	}
}

// joined is a coordinator whose hub node listens on its gateway port 49135, that listener, and a client that sees the gateway.
func joined(t *testing.T) (*Coordinator, net.Listener, *meshnet.Mesh) {
	t.Helper()
	c := newCoordinator(t, t.TempDir())
	gateway, client := joinedTo(t, c)
	return c, gateway, client
}

// joinedTo is joined for a coordinator the test built.
func joinedTo(t *testing.T, c *Coordinator) (net.Listener, *meshnet.Mesh) {
	t.Helper()
	admission := httptest.NewServer(relay.AdmitHandler(c.Known))
	t.Cleanup(admission.Close)
	derp := relay.NewWithKey(c.RelayKey(), admission.URL)
	t.Cleanup(func() { derp.Close() })
	mux := http.NewServeMux()
	c.Handle(mux)
	relay.Handle(mux, derp)
	front := httptest.NewServer(mux)
	t.Cleanup(front.Close)
	shim, err := meshnet.NewShim(front.URL, c.Public())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { shim.Close() })
	ls, err := c.Listen(t.Context(), shim.URL(), 49135)
	if err != nil {
		t.Fatal(err)
	}
	c.SetGatewayPort(49135)
	client, err := meshnet.UpUserspace(t.TempDir(), front.URL, rand.Text(), "laptop", c.Public())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { client.Close() })
	for deadline := time.Now().Add(10 * time.Second); ; time.Sleep(10 * time.Millisecond) {
		if port, up := client.GatewayPort(); client.Self().IsValid() && up && port == 49135 {
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("timed out waiting for the client's address and gateway")
		}
	}
	return ls[0], client
}

// peerAPIPort is the network library's first choice of peer API port for addr.
func peerAPIPort(addr netip.Addr) uint16 {
	a := addr.As16()
	return 32768 | uint16(crc32.ChecksumIEEE(a[13:]))
}

// reportedPeerAPIPort is the peer API port the hub node's map shows for name, or 0.
func reportedPeerAPIPort(c *Coordinator, name string) uint16 {
	for _, peer := range c.backend.backend.NetMapWithPeers().Peers {
		for _, s := range peer.Hostinfo().Services().All() {
			if peer.Name() == name && s.Proto == tailcfg.PeerAPI6 {
				return s.Port
			}
		}
	}
	return 0
}

// Security: over the mesh a joined client opens the hub node's gateway port and nothing else, the backend's peer API included.
func TestClientReachesOnlyTheHubGateway(t *testing.T) {
	c, gateway, client := joined(t)
	if c.backend.sys.Tun.Get().PeerAPIPort != nil {
		t.Error("the hub node lets peer API connections past its packet filter")
	}
	hub := c.backend.address
	// dial gives up after 3 s: a flow the packet filter drops gets no answer at all.
	dial := func(port uint16) error {
		ctx, cancel := context.WithTimeout(t.Context(), 3*time.Second)
		defer cancel()
		conn, err := client.Dial(ctx, "tcp", netip.AddrPortFrom(hub, port).String())
		if err == nil {
			conn.Close()
		}
		return err
	}
	// opened waits for a port to open: the hub node applies a filter a moment after the coordinator sends it.
	opened := func(port uint16) bool {
		for deadline := time.Now().Add(10 * time.Second); dial(port) != nil; {
			if time.Now().After(deadline) {
				return false
			}
		}
		return true
	}
	if !opened(49135) {
		t.Fatal("the gateway port did not open")
	}
	conn, err := gateway.Accept()
	if err != nil {
		t.Fatalf("the gateway listener: %v", err)
	}
	conn.Close()
	api := peerAPIPort(hub)
	if buildfeatures.HasPeerAPIServer {
		// The port this build serves, so the dial below attacks a peer API that answers.
		for deadline := time.Now().Add(10 * time.Second); ; time.Sleep(10 * time.Millisecond) {
			if port, ok := c.backend.backend.GetPeerAPIPort(hub); ok {
				api = port
				break
			}
			if time.Now().After(deadline) {
				t.Fatal("timed out waiting for the hub node's peer API port")
			}
		}
	}
	if err := dial(api); err == nil {
		t.Errorf("the client opened the hub node's peer API port %d", api)
	}
	second, err := c.ListenTCP(49200)
	if err != nil {
		t.Fatal(err)
	}
	defer second.Close()
	if err := dial(49200); err == nil {
		t.Error("the client opened a hub node port that is not the gateway")
	}
	// The filter follows the gateway port: the new one opens, and the old one closes with its listener still bound.
	c.SetGatewayPort(49200)
	if !opened(49200) {
		t.Fatal("the new gateway port did not open")
	}
	if err := dial(49135); err == nil {
		t.Fatal("the client still opened the old gateway port")
	}
}

// On an idle mesh the keepalive is the hub's only map response, so the 10-minute rule holds only while the client's network library reports its time too.
func TestClientHearsAKeepalive(t *testing.T) {
	clock := asyncClock{tstest.NewClock(tstest.ClockOpts{})}
	c, err := newWithClock(t.TempDir(), func(string, string) (string, error) { return "", nil }, clock)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { c.Close() })
	_, client := joinedTo(t, c)
	// The lock holds back every map response but the keepalive, which is written without it.
	c.mu.Lock()
	defer c.mu.Unlock()
	heard := client.HubHeard()
	for { // a response written before the lock may still be on its way
		time.Sleep(100 * time.Millisecond)
		now := client.HubHeard()
		if now.Equal(heard) {
			break
		}
		heard = now
	}
	clock.Advance(time.Minute)
	for deadline := time.Now().Add(10 * time.Second); !client.HubHeard().After(heard); time.Sleep(10 * time.Millisecond) {
		if time.Now().After(deadline) {
			t.Fatal("the client did not hear the keepalive")
		}
	}
}

// A timer does not count the time a machine sleeps, so the hub hears the wake; a link change that is no wake does not run the hook.
func TestWakeRunsTheHook(t *testing.T) {
	c, _, _ := joined(t)
	var woke atomic.Int32
	c.OnWake(func() { woke.Add(1) })
	state := c.backend.monitor.InterfaceState()
	changes := eventbus.Publish[netmon.ChangeDelta](c.backend.sys.Bus.Get().Client("test"))
	for _, jump := range []time.Duration{0, time.Minute} {
		delta, err := netmon.NewChangeDelta(state, state, jump, false)
		if err != nil {
			t.Fatal(err)
		}
		changes.Publish(*delta)
	}
	for deadline := time.Now().Add(10 * time.Second); woke.Load() == 0; time.Sleep(10 * time.Millisecond) {
		if time.Now().After(deadline) {
			t.Fatal("a wake did not run the hook")
		}
	}
	// Had the change with no jump run the hook too, there are two runs: the wait lets the second one land.
	time.Sleep(100 * time.Millisecond)
	if n := woke.Load(); n != 1 {
		t.Fatalf("the hook ran %d times, want once, for the wake", n)
	}
}

// Security: the hub is a client's only peer and must not reach the client host's services through the client's netstack.
func TestHubCannotReachClientHostLoopback(t *testing.T) {
	c, gateway, client := joined(t)
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
	// The forward proves the path between hub and client, so a dial below that gets no answer was dropped by the client's filter.
	forward, token, err := client.Forward()
	if err != nil {
		t.Fatal(err)
	}
	local, err := net.Dial("tcp", strings.TrimPrefix(forward, "ws://"))
	if err != nil {
		t.Fatal(err)
	}
	defer local.Close()
	local.SetDeadline(time.Now().Add(30 * time.Second))
	req, _ := http.NewRequest("GET", forward, nil)
	req.Header.Set(meshnet.TokenHeader, token)
	if err := req.Write(local); err != nil {
		t.Fatal(err)
	}
	remote, err := gateway.Accept()
	if err != nil {
		t.Fatal(err)
	}
	defer remote.Close()
	remote.SetDeadline(time.Now().Add(30 * time.Second))
	if _, err := http.ReadRequest(bufio.NewReader(remote)); err != nil {
		t.Fatalf("hub read: %v", err)
	}
	got := make([]byte, 2)
	if _, err := remote.Write([]byte("ok")); err != nil {
		t.Fatal(err)
	}
	if _, err := io.ReadFull(local, got); err != nil || string(got) != "ok" {
		t.Fatalf("client read %q: %v", got, err)
	}
	stack := c.backend.stack
	// The backend's peer API too: to it the hub is the owner, and /dns-query would resolve through the client host.
	api := peerAPIPort(client.Self())
	for deadline := time.Now().Add(10 * time.Second); buildfeatures.HasPeerAPIServer; time.Sleep(10 * time.Millisecond) {
		if port := reportedPeerAPIPort(c, "laptop"); port != 0 {
			api = port
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("timed out waiting for the client's peer API port in the hub node's map")
		}
	}
	for _, port := range []uint16{uint16(service.Addr().(*net.TCPAddr).Port), api} {
		ctx, cancel := context.WithTimeout(t.Context(), 3*time.Second)
		conn, err := stack.DialContextTCP(ctx, netip.AddrPortFrom(client.Self(), port))
		cancel()
		if err == nil {
			conn.Close()
			t.Errorf("hub opened TCP port %d on the client", port)
		}
	}
	// Undenied UDP to a local address is forwarded to 127.0.0.1:port.
	udp, err := stack.DialContextUDP(t.Context(), netip.AddrPortFrom(client.Self(), uint16(datagrams.LocalAddr().(*net.UDPAddr).Port)))
	if err != nil {
		t.Fatal(err)
	}
	defer udp.Close()
	if _, err := udp.Write([]byte("x")); err != nil {
		t.Fatal(err)
	}
	datagrams.SetReadDeadline(time.Now().Add(time.Second))
	if _, _, err := datagrams.ReadFrom(make([]byte, 1)); err == nil {
		t.Fatal("hub reached a 127.0.0.1 UDP service on the client")
	}
	select {
	case <-accepted:
		t.Fatal("loopback service accepted the hub")
	default:
	}
}
