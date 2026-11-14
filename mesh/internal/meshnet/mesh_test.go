package meshnet

import (
	"bufio"
	"context"
	"crypto/rand"
	"errors"
	"hash/crc32"
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
	"tailscale.com/feature/buildfeatures"
	"tailscale.com/tailcfg"
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
		forward, token, err := m.Forward()
		if err != nil {
			t.Fatal(err)
		}
		forward = strings.TrimPrefix(forward, "ws://")
		upgrade := func(path, token string) net.Conn {
			t.Helper()
			c, err := net.Dial("tcp", forward)
			if err != nil {
				t.Fatal(err)
			}
			c.SetReadDeadline(time.Now().Add(10 * time.Second))
			req, _ := http.NewRequest("GET", "http://"+forward+path, nil)
			req.Header.Set("Connection", "Upgrade")
			req.Header.Set("Upgrade", "websocket")
			if token != "" {
				req.Header.Set(TokenHeader, token)
			}
			if err := req.Write(c); err != nil {
				t.Fatal(err)
			}
			return c
		}
		// Refused before anything is dialed: the gateway's first connection is the good one below.
		for _, bad := range []string{"", "wrong"} {
			c := upgrade("/refused", bad)
			if _, err := c.Read(make([]byte, 1)); err != io.EOF {
				t.Fatalf("token %q not refused: %v", bad, err)
			}
			c.Close()
		}
		// A head over the limit is refused even with the token, before it is read whole.
		big, err := net.Dial("tcp", forward)
		if err != nil {
			t.Fatal(err)
		}
		big.SetReadDeadline(time.Now().Add(10 * time.Second))
		go big.Write([]byte("GET /oversized HTTP/1.1\r\nHost: x\r\n" + TokenHeader + ": " + token + "\r\nX-Pad: " + strings.Repeat("a", 128<<10) + "\r\n\r\n"))
		if _, err := big.Read(make([]byte, 1)); err == nil || errors.Is(err, os.ErrDeadlineExceeded) {
			t.Fatalf("oversized head not refused: %v", err)
		}
		big.Close()
		local := upgrade("/accepted", token)
		remote, err := gateway[0].Accept()
		if err != nil {
			t.Fatal(err)
		}
		remote.SetReadDeadline(time.Now().Add(10 * time.Second))
		if got := netip.MustParseAddrPort(remote.RemoteAddr().String()).Addr(); got != m.Self() {
			t.Fatalf("gateway saw %s, want the client's tailnet address %s", got, m.Self())
		}
		head, err := http.ReadRequest(bufio.NewReader(remote))
		if err != nil || head.URL.Path != "/accepted" || head.Header.Get("Upgrade") != "websocket" || len(head.Header.Values(TokenHeader)) != 0 {
			t.Fatalf("hub read %+v: %v", head, err)
		}
		buf := make([]byte, 2)
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
	if l, err := m.ListenPeers(); err == nil {
		l.Close()
		t.Fatal("a machine with no address opened the peer listener")
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

func TestPeerConstantsMatchCoordinator(t *testing.T) {
	// Below 16000 neither the backend's peer API nor an outgoing connection can hold the port.
	if port, tag := PeerPort, peerTag; port != coordinator.PeerPort || tag != coordinator.PeerTag || port >= 16000 {
		t.Fatalf("port %d and tag %q, the coordinator has %d and %q", port, tag, coordinator.PeerPort, coordinator.PeerTag)
	}
}

// seen is an OnChange hook and the channel that hears each run in which ok held.
func seen(ok func() bool) (func(), <-chan struct{}) {
	ch := make(chan struct{}, 1)
	return func() {
		if ok() {
			select {
			case ch <- struct{}{}:
			default:
			}
		}
	}, ch
}

func await(t *testing.T, ch <-chan struct{}, what string) {
	t.Helper()
	select {
	case <-ch:
	case <-time.After(10 * time.Second):
		t.Fatal("timed out waiting for " + what)
	}
}

func upPeer(t *testing.T, c *coordinator.Coordinator, publicURL, dir, name string) *Mesh {
	t.Helper()
	m, err := UpUserspace(dir, publicURL, rand.Text(), name, c.Public())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { m.Close() })
	waitFor(t, name+" address", func() bool { return m.Self().IsValid() })
	return m
}

// dialPeer gives up after wait: a flow the filter drops gets no answer at all.
func dialPeer(m *Mesh, to netip.Addr, port uint16, wait time.Duration) (net.Conn, error) {
	ctx, cancel := context.WithTimeout(context.Background(), wait)
	defer cancel()
	return m.Dial(ctx, "tcp", netip.AddrPortFrom(to, port).String())
}

// routed is whether m's map says where name's relay is: a first handshake sent before that is lost and WireGuard retries 5 s later.
func routed(m *Mesh, name string) bool {
	for _, p := range m.lb.NetMapWithPeers().Peers {
		if p.Name() == name {
			return p.HomeDERP() != 0
		}
	}
	return false
}

type pair struct {
	c                *coordinator.Coordinator
	a, b             *Mesh
	l                net.Listener
	dialed, accepted net.Conn
}

// peerPair joins alpha and beta, turns peer to peer on at both and opens a connection from alpha to beta's listener.
func peerPair(t *testing.T) pair {
	t.Helper()
	c, publicURL := newTestControl(t)
	p := pair{c: c, a: upPeer(t, c, publicURL, t.TempDir(), "alpha"), b: upPeer(t, c, publicURL, t.TempDir(), "beta")}
	if _, ok := p.a.Peer("beta"); ok {
		t.Fatal("alpha sees beta before either asked")
	}
	hook, appeared := seen(func() bool { _, ok := p.a.Peer("beta"); return ok })
	p.a.OnChange(hook)
	for _, m := range []*Mesh{p.a, p.b} {
		if err := m.SetPeerToPeer(true); err != nil {
			t.Fatal(err)
		}
	}
	await(t, appeared, "alpha's OnChange to run with beta in its map")
	waitFor(t, "alpha's route to beta", func() bool { return routed(p.a, "beta") })
	waitFor(t, "beta's route to alpha", func() bool { return routed(p.b, "alpha") })
	var err error
	if p.l, err = p.b.ListenPeers(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { p.l.Close() })
	if p.dialed, err = dialPeer(p.a, p.b.Self(), PeerPort, 10*time.Second); err != nil {
		t.Fatalf("alpha cannot reach beta's listener: %v", err)
	}
	t.Cleanup(func() { p.dialed.Close() })
	if p.accepted, err = p.l.Accept(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { p.accepted.Close() })
	return p
}

// Two machines that both turned peer to peer on reach each other on PeerPort alone, and the target names its caller by address.
func TestPeerToPeer(t *testing.T) {
	conns := peerConns
	peerConns = 1
	t.Cleanup(func() { peerConns = conns })
	p := peerPair(t)
	from := netip.MustParseAddrPort(p.accepted.RemoteAddr().String())
	if name, ok := p.b.WhoIs(from); !ok || name != "alpha" || from.Addr() != p.a.Self() {
		t.Fatalf("beta names its caller %q (%v) at %s, want alpha at %s", name, ok, from, p.a.Self())
	}
	if name, ok := p.b.WhoIs(netip.AddrPortFrom(from.Addr(), 0)); !ok || name != "alpha" {
		t.Fatalf("beta names the bare address %q (%v)", name, ok)
	}
	if name, ok := p.b.WhoIs(netip.MustParseAddrPort("[fd00::1]:9")); ok {
		t.Fatalf("beta names an address outside its map %q", name)
	}
	go p.dialed.Write([]byte("hi"))
	p.accepted.SetReadDeadline(time.Now().Add(10 * time.Second))
	buf := make([]byte, 2)
	if _, err := io.ReadFull(p.accepted, buf); err != nil || string(buf) != "hi" {
		t.Fatalf("beta read %q: %v", buf, err)
	}
	// The cap on open connections: one past it waits for a slot instead of being served.
	second, err := dialPeer(p.a, p.b.Self(), PeerPort, 10*time.Second)
	if err != nil {
		t.Fatal(err)
	}
	defer second.Close()
	served := make(chan struct{}, 1)
	go func() { p.l.Accept(); served <- struct{}{} }()
	select {
	case <-served:
		t.Fatal("beta took a connection past its cap")
	case <-time.After(200 * time.Millisecond):
	}
	p.accepted.Close()
	await(t, served, "the closed connection's slot")
	// The filter alone keeps a visible peer from the backend's peer API, so that is the port to dial: the network library's first choice, or the one beta serves when this build has the server.
	a := p.b.Self().As16()
	api := 32768 | uint16(crc32.ChecksumIEEE(a[13:]))
	if buildfeatures.HasPeerAPIServer {
		waitFor(t, "beta's peer API port in alpha's map", func() bool {
			for _, peer := range p.a.lb.NetMapWithPeers().Peers {
				for _, s := range peer.Hostinfo().Services().All() {
					if peer.Name() == "beta" && s.Proto == tailcfg.PeerAPI6 {
						api = s.Port
						return true
					}
				}
			}
			return false
		})
	}
	if c, err := dialPeer(p.a, p.b.Self(), api, 2*time.Second); err == nil {
		c.Close()
		t.Fatalf("alpha opened beta's peer API port %d", api)
	} else if c != nil {
		t.Fatal("a failed dial returned a connection")
	}
	// Only an address binds a call to the map, so a name is refused before anything is dialed.
	if _, err := p.a.Dial(t.Context(), "tcp", "beta:9136"); err == nil || errors.Is(err, context.DeadlineExceeded) {
		t.Fatalf("alpha dialed a name: %v", err)
	}
	// A closed listener leaves the port allowed by the filter and claimed by nothing.
	p.l.Close()
	if c, err := dialPeer(p.a, p.b.Self(), PeerPort, 2*time.Second); err == nil {
		c.Close()
		t.Fatal("alpha opened beta's port with no listener on it")
	}
	if err := p.b.SetPeerToPeer(false); err != nil {
		t.Fatal(err)
	}
	waitFor(t, "alpha to lose beta, which turned peer to peer off", func() bool { _, ok := p.a.Peer("beta"); return !ok })
	waitFor(t, "beta to lose alpha", func() bool { _, ok := p.b.Peer("alpha"); return !ok })
}

// These connections have no keepalive: a machine the hub removed must already be gone from the lookups when OnChange runs, or nothing ends its calls.
func TestPeerRemovedWithConnectionOpen(t *testing.T) {
	t.Run("target", func(t *testing.T) {
		p := peerPair(t)
		to := netip.MustParseAddrPort(p.dialed.RemoteAddr().String()).Addr()
		hook, gone := seen(func() bool { addr, ok := p.a.Peer("beta"); return !ok || addr != to })
		p.a.OnChange(hook)
		if err := p.c.Remove("beta"); err != nil {
			t.Fatal(err)
		}
		await(t, gone, "alpha's OnChange to run with beta out of its map")
	})
	t.Run("caller", func(t *testing.T) {
		p := peerPair(t)
		from := netip.MustParseAddrPort(p.accepted.RemoteAddr().String()).Addr()
		hook, gone := seen(func() bool { _, ok := p.b.WhoIs(netip.AddrPortFrom(from, 0)); return !ok })
		p.b.OnChange(hook)
		if err := p.c.Remove("alpha"); err != nil {
			t.Fatal(err)
		}
		await(t, gone, "beta's OnChange to run with alpha's address unnamed")
		await(t, p.a.Removed(), "alpha to hear its removal")
		if c, err := dialPeer(p.a, p.b.Self(), PeerPort, 2*time.Second); err == nil {
			c.Close()
			t.Fatal("a removed machine opened beta's listener")
		}
	})
}

// A restart begins without the tag, so a machine is never shown to peers before its listener is open again.
func TestRestartDropsPeerTag(t *testing.T) {
	c, publicURL := newTestControl(t)
	dir := t.TempDir()
	a, b := upPeer(t, c, publicURL, dir, "alpha"), upPeer(t, c, publicURL, t.TempDir(), "beta")
	for _, m := range []*Mesh{a, b} {
		if err := m.SetPeerToPeer(true); err != nil {
			t.Fatal(err)
		}
	}
	waitFor(t, "beta to see alpha", func() bool { _, ok := b.Peer("alpha"); return ok })
	l, err := a.ListenPeers()
	if err != nil {
		t.Fatal(err)
	}
	a.Close()
	closed := make(chan struct{}, 1)
	go func() { l.Accept(); closed <- struct{}{} }()
	await(t, closed, "alpha's listener to close with its mesh")
	a = upPeer(t, c, publicURL, dir, "alpha")
	if tags := a.lb.Prefs().AdvertiseTags(); tags.Len() != 0 {
		t.Fatalf("restarted with tags %v", tags)
	}
	waitFor(t, "beta to lose alpha, whose restart sent no tag", func() bool { _, ok := b.Peer("alpha"); return !ok })
}

// The 10-minute rule reads this: only a map response of the hub moves the time, with nothing reading it, and nothing moves it with the hub out of reach.
func TestHubHeard(t *testing.T) {
	c, publicURL := newTestControl(t)
	m := upPeer(t, c, publicURL, t.TempDir(), "alpha")
	waitFor(t, "the hub's first map response to be heard", func() bool { return m.heard.Load() != nil })
	first := m.HubHeard()
	if strings.Contains(first.String(), "m=") {
		t.Fatalf("hub heard %s keeps a monotonic reading, which leaves out the time asleep", first)
	}
	if err := m.SetPeerToPeer(true); err != nil { // the hub answers a new tag with a map response
		t.Fatal(err)
	}
	waitFor(t, "the hub's answer to move the time", func() bool { return m.HubHeard().After(first) })
	m.shim.Close() // the hub is out of reach
	waitFor(t, "the control stream to close", func() bool { return !m.sys.HealthTracker.Get().GetInPollNetMap() })
	last := m.HubHeard()
	time.Sleep(50 * time.Millisecond)
	if got := m.HubHeard(); !got.Equal(last) {
		t.Fatalf("hub heard moved from %s to %s with the control stream closed", last, got)
	}
	// The runner turns the setting on and off in its one loop, so asking must not wait for a hub out of reach.
	asked := make(chan struct{}, 1)
	go func() { m.SetPeerToPeer(false); m.SetPeerToPeer(true); asked <- struct{}{} }()
	await(t, asked, "the tag to be withdrawn and set with the hub out of reach")
}

// The backend's link handler notifies nothing, and the client reads its path to the hub in OnChange.
func TestALinkChangeRunsOnChange(t *testing.T) {
	c, publicURL := newTestControl(t)
	m := upPeer(t, c, publicURL, t.TempDir(), "alpha")
	// With the watcher stopped only the link change can run the hook: the engine notifies the backend too when the change moves its endpoints.
	m.unwatch()
	<-m.watched
	hook, ran := seen(func() bool { return true })
	m.OnChange(hook)
	m.monitor.InjectEvent()
	await(t, ran, "OnChange to run for the link change")
}
