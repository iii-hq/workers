package client

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"net"
	"net/http"
	"net/http/httptest"
	"net/netip"
	"os"
	"slices"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/meshnet"
	"github.com/iii-hq/workers/mesh/internal/node"
)

var (
	hubAddr     = netip.MustParseAddr("2001:db8::1")
	alphaAddr   = netip.MustParseAddr("2001:db8::a")
	betaAddr    = netip.MustParseAddr("2001:db8::b")
	gammaAddr   = netip.MustParseAddr("2001:db8::c")
	unknownAddr = netip.MustParseAddr("2001:db8::d")
)

const testVersion = "0.3.0"

const channelRef = `{"data":{"reader":{"channel_id":"c","access_key":"k","direction":"read"}}}`

// fakeNet is a directNet over loopback. Its listener reports each connection as
// coming from the address its dialer was given, so two machines need no second
// loopback address.
type fakeNet struct {
	mu     sync.Mutex
	self   netip.Addr            // where this machine's dials come from
	to     *fakeNet              // the machine every Dial reaches
	plain  string                // without to: a loopback address standing in for the target
	listen string                // loopback address of the open listener
	srcs   map[string]netip.Addr // by the dialer's loopback address: the source the listener reports
	names  map[netip.Addr]string // WhoIs
	peers  map[string]netip.Addr // Peer
	paths  []meshnet.PeerStatus  // Peers
	heard  time.Time             // HubHeard
	steps  []string              // what SetDirect did, in order
	dialed []string              // the addresses Dial was asked for

	failListen, failAsk, failWithdraw error
}

func (n *fakeNet) set(f func()) {
	n.mu.Lock()
	defer n.mu.Unlock()
	f()
}

func (n *fakeNet) did() []string {
	n.mu.Lock()
	defer n.mu.Unlock()
	return slices.Clone(n.steps)
}

func (n *fakeNet) dials() int {
	n.mu.Lock()
	defer n.mu.Unlock()
	return len(n.dialed)
}

func (n *fakeNet) SetPeerToPeer(on bool) error {
	n.mu.Lock()
	defer n.mu.Unlock()
	if on {
		n.steps = append(n.steps, "ask")
		return n.failAsk
	}
	n.steps = append(n.steps, "withdraw")
	return n.failWithdraw
}

func (n *fakeNet) ListenPeers() (net.Listener, error) {
	n.mu.Lock()
	defer n.mu.Unlock()
	n.steps = append(n.steps, "listen")
	if n.failListen != nil {
		return nil, n.failListen
	}
	l, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return nil, err
	}
	n.listen = l.Addr().String()
	return &fakeListener{Listener: l, n: n}, nil
}

func (n *fakeNet) Dial(_ context.Context, _, addr string) (net.Conn, error) {
	n.mu.Lock()
	n.dialed = append(n.dialed, addr)
	to, plain := n.to, n.plain
	n.mu.Unlock()
	if to != nil {
		return to.connect(n.self)
	}
	return net.Dial("tcp", plain)
}

func (n *fakeNet) WhoIs(from netip.AddrPort) (string, bool) {
	n.mu.Lock()
	defer n.mu.Unlock()
	name, ok := n.names[from.Addr()]
	return name, ok
}

func (n *fakeNet) Peer(name string) (netip.Addr, bool) {
	n.mu.Lock()
	defer n.mu.Unlock()
	a, ok := n.peers[name]
	return a, ok
}

func (n *fakeNet) Peers() []meshnet.PeerStatus {
	n.mu.Lock()
	defer n.mu.Unlock()
	return n.paths
}

func (n *fakeNet) HubHeard() time.Time {
	n.mu.Lock()
	defer n.mu.Unlock()
	return n.heard
}

// connect opens a connection that n's listener reports as coming from the machine at from.
func (n *fakeNet) connect(from netip.Addr) (net.Conn, error) {
	n.mu.Lock() // held across the dial: Accept looks the source up under it
	defer n.mu.Unlock()
	c, err := net.Dial("tcp", n.listen)
	if err != nil {
		return nil, err
	}
	if n.srcs == nil {
		n.srcs = map[string]netip.Addr{}
	}
	n.srcs[c.LocalAddr().String()] = from
	return c, nil
}

type fakeListener struct {
	net.Listener
	n      *fakeNet
	closed sync.Once
}

func (l *fakeListener) Accept() (net.Conn, error) {
	c, err := l.Listener.Accept()
	if err != nil {
		return nil, err
	}
	l.n.mu.Lock()
	from := l.n.srcs[c.RemoteAddr().String()]
	l.n.mu.Unlock()
	return &fakeConn{Conn: c, from: net.TCPAddrFromAddrPort(netip.AddrPortFrom(from, 40000))}, nil
}

func (l *fakeListener) Close() error {
	l.closed.Do(func() { l.n.set(func() { l.n.steps = append(l.n.steps, "close") }) })
	return l.Listener.Close()
}

type fakeConn struct {
	net.Conn
	from net.Addr
}

func (c *fakeConn) RemoteAddr() net.Addr { return c.from }

// try makes one direct call to n's listener as the machine at from, the way a
// caller does: it waits for "100 Continue" before sending the body, so read
// says whether the target read it.
func (n *fakeNet) try(from netip.Addr, method, target, version, body string) (status int, code string, raw []byte, read bool, err error) {
	c, err := n.connect(from)
	if err != nil {
		return 0, "", nil, false, err
	}
	defer c.Close()
	_ = c.SetDeadline(time.Now().Add(20 * time.Second))
	head := method + " " + target + " HTTP/1.1\r\nHost: beta\r\nConnection: close\r\n"
	if version != "" {
		head += "X-Mesh-Version: " + version + "\r\n"
	}
	if body != "" {
		head += "Expect: 100-continue\r\n"
	}
	if _, err = fmt.Fprintf(c, "%sContent-Length: %d\r\n\r\n", head, len(body)); err != nil {
		return 0, "", nil, false, err
	}
	br := bufio.NewReader(c)
	res, err := http.ReadResponse(br, nil)
	if err == nil && res.StatusCode == http.StatusContinue {
		read = true
		if _, err = io.WriteString(c, body); err == nil {
			res, err = http.ReadResponse(br, nil)
		}
	}
	if err != nil {
		return 0, "", nil, read, err
	}
	raw, err = io.ReadAll(res.Body)
	var e struct{ Code string }
	_ = json.Unmarshal(raw, &e)
	return res.StatusCode, e.Code, raw, read, err
}

func (n *fakeNet) send(t *testing.T, from netip.Addr, target, body string) (status int, code string, raw []byte, read bool) {
	t.Helper()
	status, code, raw, read, err := n.try(from, "POST", target, testVersion, body)
	if err != nil {
		t.Fatalf("%s: %v", target, err)
	}
	return status, code, raw, read
}

// probe is the worker beta shares.
type probe struct {
	release chan struct{}   // closed to let probe::wait return
	ended   chan error      // what ended each probe::wait
	big     json.RawMessage // what probe::big returns
}

// let releases one probe::wait.
func (p *probe) let(t *testing.T) {
	t.Helper()
	select {
	case p.release <- struct{}{}:
	case <-time.After(5 * time.Second):
		t.Fatal("no function on the target is waiting")
	}
}

// ends is what ended the probe::wait that returned last.
func (p *probe) ends(t *testing.T) error {
	t.Helper()
	select {
	case err := <-p.ended:
		return err
	case <-time.After(5 * time.Second):
		t.Fatal("the function on the target is still waiting")
		return nil
	}
}

func (p *probe) call(ctx context.Context, r iii.CallRequest) (json.RawMessage, error) {
	switch r.FunctionID {
	case "probe::echo":
		return r.Payload, nil
	case "probe::void":
		return nil, nil
	case "probe::fail":
		return nil, &iii.InvocationError{Code: "PROBE_FAILED", Message: "no"}
	case "probe::plain":
		return nil, errors.New("boom")
	case "probe::refs":
		return json.RawMessage(channelRef), nil
	case "probe::big":
		return p.big, nil
	case "probe::slow":
		select {
		case <-time.After(3 * directIO):
			return json.RawMessage(`"slow"`), nil
		case <-ctx.Done():
			return nil, ctx.Err()
		}
	case "probe::wait":
		select {
		case <-p.release:
			p.ended <- nil
			return json.RawMessage(`"released"`), nil
		case <-ctx.Done():
			p.ended <- ctx.Err()
			return nil, ctx.Err()
		}
	}
	return nil, fmt.Errorf("unexpected call %s", r.FunctionID)
}

var probeFunctions = []string{"probe::echo", "probe::void", "probe::fail", "probe::plain", "probe::refs", "probe::big", "probe::slow", "probe::wait"}

// newTarget is machine beta with peer to peer on: it shares the probe worker, and
// its engine also lists ids that must never be callable.
func newTarget(t *testing.T) (*Bridge, *fakeNet, *fakeLocal, *probe) {
	t.Helper()
	t.Setenv("HOME", t.TempDir()) // the SDK writes an iii ps marker per client
	p := &probe{release: make(chan struct{}), ended: make(chan error, 2*MaxCallsInFlight)}
	rows := []map[string]any{row("shell::run", "local", "ide")} // a worker beta does not share
	for _, id := range append(slices.Clone(probeFunctions), "mesh::status", "approval::resolve", "peer::desk::coder::read-file", "hub::shell::exec") {
		rows = append(rows, row(id, "local", "probe"))
	}
	local := engine(&rows)
	list := local.fn
	local.ctxFn = func(ctx context.Context, r iii.CallRequest) (json.RawMessage, error) {
		if strings.HasPrefix(r.FunctionID, "engine::") {
			return list(r)
		}
		return p.call(ctx, r)
	}
	b := NewBridge(local, "local")
	t.Cleanup(b.Close)
	if err := b.SetRuntime(context.Background(), node.Runtime{Node: "beta", Namespace: "hub"}); err != nil { // no hub connection: the gate needs none
		t.Fatal(err)
	}
	b.SetCatalog(node.Catalog{Rev: "r1", Functions: []node.CatalogEntry{{ID: "desk::coder::read-file", Owner: "desk"}, {ID: "shell::exec", Owner: "hub"}}})
	if err := b.Sync(context.Background(), []string{"probe"}); err != nil {
		t.Fatal(err)
	}
	n := &fakeNet{self: betaAddr, heard: time.Now(), names: map[netip.Addr]string{hubAddr: "hub", alphaAddr: "alpha", betaAddr: "beta", gammaAddr: "gamma"}}
	if err := b.SetDirect(n, testVersion); err != nil {
		t.Fatal(err)
	}
	return b, n, local, p
}

// newCaller is machine alpha with peer to peer on towards nb: it holds mirrors of
// what beta and the hub share. Its hub connection is down unless hubURL is set.
func newCaller(t *testing.T, nb *fakeNet, hubURL string) (*Bridge, *fakeNet, *fakeLocal) {
	t.Helper()
	t.Setenv("HOME", t.TempDir())
	local := engine(&[]map[string]any{})
	a := NewBridge(local, "local")
	t.Cleanup(a.Close)
	if err := a.SetRuntime(context.Background(), node.Runtime{Node: "alpha", Namespace: "hub", URL: hubURL}); err != nil {
		t.Fatal(err)
	}
	catalog := node.Catalog{Rev: "r1", Functions: []node.CatalogEntry{{ID: "shell::exec", Owner: "hub"}}}
	for _, id := range probeFunctions {
		catalog.Functions = append(catalog.Functions, node.CatalogEntry{ID: "beta::" + id, Owner: "beta"})
	}
	a.SetCatalog(catalog)
	n := &fakeNet{self: alphaAddr, to: nb, heard: time.Now(), peers: map[string]netip.Addr{"hub": hubAddr, "beta": betaAddr}}
	if err := a.SetDirect(n, testVersion); err != nil {
		t.Fatal(err)
	}
	return a, n, local
}

func codeOf(err error) string {
	var ie *iii.InvocationError
	if errors.As(err, &ie) {
		return ie.Code
	}
	return ""
}

func (d *direct) running(from netip.Addr) int {
	d.mu.Lock()
	defer d.mu.Unlock()
	return d.inFlight[from]
}

func (d *direct) open(from netip.Addr) int {
	d.mu.Lock()
	defer d.mu.Unlock()
	n := 0
	for _, a := range d.conns {
		if a == from {
			n++
		}
	}
	return n
}

// idle says no call runs on d and no slot is left behind.
func (d *direct) idle() bool {
	d.mu.Lock()
	defer d.mu.Unlock()
	return len(d.inFlight) == 0
}

func shorten[T any](t *testing.T, v *T, to T) {
	old := *v
	*v = to
	t.Cleanup(func() { *v = old })
}

func TestDirectCallRunsWhatIsShared(t *testing.T) {
	b, n, local, _ := newTarget(t)
	payload := `{"b": [1, 2.50],"a":"x"}`
	status, _, raw, read := n.send(t, alphaAddr, "/call?function=probe::echo", payload)
	if status != 200 || string(raw) != payload || !read {
		t.Fatalf("echo: %d %s, body read %v", status, raw, read)
	}
	c := local.called("probe::echo")[0]
	if c.Namespace != "local" || c.Timeout != 10*time.Minute || string(c.Payload) != payload || c.Metadata != nil || c.Action != nil {
		t.Fatalf("local call %+v", c)
	}
	if status, code, raw, _ := n.send(t, alphaAddr, "/call?function=probe::fail", payload); status != 500 || code != "PROBE_FAILED" || !strings.Contains(string(raw), `"message":"no"`) {
		t.Fatalf("the function's own error: %d %s", status, raw)
	}
	if status, code, raw, _ := n.send(t, alphaAddr, "/call?function=probe::plain", payload); status != 500 || code != "invocation_failed" || !strings.Contains(string(raw), `"message":"boom"`) {
		t.Fatalf("a plain error: %d %s", status, raw)
	}
	// No payload is nil to the SDK, which sends {}; an empty one would be invalid JSON.
	if status, _, raw, _ := n.send(t, alphaAddr, "/call?function=probe::void", ""); status != 200 || len(raw) != 0 || local.called("probe::void")[0].Payload != nil {
		t.Fatalf("no payload and no result: %d %q %+v", status, raw, local.called("probe::void"))
	}
	if !b.direct.idle() {
		t.Fatal("finished calls left a slot taken")
	}
}

func TestDirectGateRefusesBeforeTheBody(t *testing.T) {
	b, n, local, _ := newTarget(t)
	old := time.Now().Add(-directGrace - time.Minute)
	for _, c := range []struct {
		what, method, target, version string
		from                          netip.Addr
		heard                         time.Time
		status                        int
		code                          string
	}{
		{"an address the map does not name", "POST", "/call?function=probe::echo", testVersion, unknownAddr, time.Now(), 403, "FORBIDDEN"},
		{"the hub", "POST", "/call?function=probe::echo", testVersion, hubAddr, time.Now(), 403, "FORBIDDEN"},
		{"this machine's own name", "POST", "/call?function=probe::echo", testVersion, betaAddr, time.Now(), 403, "FORBIDDEN"},
		{"another method", "GET", "/call?function=probe::echo", testVersion, alphaAddr, time.Now(), 403, "FORBIDDEN"},
		{"another path", "POST", "/?function=probe::echo", testVersion, alphaAddr, time.Now(), 403, "FORBIDDEN"},
		{"a hub not heard for longer than the grace", "POST", "/call?function=probe::echo", testVersion, alphaAddr, old, 503, "mesh_offline"},
		{"a hub never heard", "POST", "/call?function=probe::echo", testVersion, alphaAddr, time.Time{}, 503, "mesh_offline"},
		{"another version", "POST", "/call?function=probe::echo", "0.2.0", alphaAddr, time.Now(), 403, "FORBIDDEN"},
		{"no version", "POST", "/call?function=probe::echo", "", alphaAddr, time.Now(), 403, "FORBIDDEN"},
		{"no function", "POST", "/call", testVersion, alphaAddr, time.Now(), 403, "FORBIDDEN"},
		{"a function that does not exist", "POST", "/call?function=probe::other", testVersion, alphaAddr, time.Now(), 403, "FORBIDDEN"},
		{"a worker that is not shared", "POST", "/call?function=shell::run", testVersion, alphaAddr, time.Now(), 403, "FORBIDDEN"},
		{"mesh::status", "POST", "/call?function=mesh::status", testVersion, alphaAddr, time.Now(), 403, "FORBIDDEN"},
		{"approval::resolve", "POST", "/call?function=approval::resolve", testVersion, alphaAddr, time.Now(), 403, "FORBIDDEN"},
		{"a mirror of another machine", "POST", "/call?function=peer::desk::coder::read-file", testVersion, alphaAddr, time.Now(), 403, "FORBIDDEN"},
		{"a mirror of the hub", "POST", "/call?function=hub::shell::exec", testVersion, alphaAddr, time.Now(), 403, "FORBIDDEN"},
		{"a declared length over the cap", "POST", "/call?function=probe::echo", testVersion, alphaAddr, time.Now(), 413, "payload_too_large"},
	} {
		n.set(func() { n.heard = c.heard })
		body := `{"x":1}`
		if c.code == "payload_too_large" {
			shorten(t, &maxDirectBody, len(body)-1)
		}
		status, code, raw, read, err := n.try(c.from, c.method, c.target, c.version, body)
		if err != nil || status != c.status || code != c.code || read {
			t.Fatalf("%s: %d %s, body read %v, %v", c.what, status, raw, read, err)
		}
	}
	local.mu.Lock()
	for _, c := range local.calls {
		if !strings.HasPrefix(c.FunctionID, "engine::") {
			t.Errorf("a refused call ran %s", c.FunctionID)
		}
	}
	local.mu.Unlock()
	if !b.direct.idle() {
		t.Fatal("refusals left a slot taken")
	}
}

// What the function gets and gives must stay on its engine, and neither side may exceed the cap.
func TestDirectRefusesChannelRefsAndLongBodies(t *testing.T) {
	b, n, local, _ := newTarget(t)
	if status, code, _, _ := n.send(t, alphaAddr, "/call?function=probe::echo", channelRef); status != 400 || code != "channels_unsupported" || len(local.called("probe::echo")) != 0 {
		t.Fatalf("a channel ref in the payload: %d %s", status, code)
	}
	if status, code, _, _ := n.send(t, alphaAddr, "/call?function=probe::refs", `{}`); status != 400 || code != "channels_unsupported" {
		t.Fatalf("a channel ref in the result: %d %s", status, code)
	}
	shorten(t, &maxDirectBody, 64)
	if status, _, _, _ := n.send(t, alphaAddr, "/call?function=probe::echo", `"`+strings.Repeat("a", 62)+`"`); status != 200 {
		t.Fatalf("a body of exactly the cap: %d", status)
	}
	// Chunked, so the target learns the length only by reading.
	client := &http.Client{Transport: &http.Transport{DisableKeepAlives: true, DialContext: func(context.Context, string, string) (net.Conn, error) { return n.connect(alphaAddr) }}}
	req, _ := http.NewRequest("POST", "http://beta/call?function=probe::echo", io.MultiReader(strings.NewReader(`"`+strings.Repeat("a", 63)+`"`)))
	req.Header.Set("X-Mesh-Version", testVersion)
	res, err := client.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	raw, _ := io.ReadAll(res.Body)
	res.Body.Close()
	if res.StatusCode != 413 || !strings.Contains(string(raw), `"payload_too_large"`) || len(local.called("probe::echo")) != 1 {
		t.Fatalf("an undeclared body over the cap: %d %s", res.StatusCode, raw)
	}
	waitUntil(t, "the slot of the refused body", b.direct.idle)
}

func TestDirectUnshareRefusesTheNextCall(t *testing.T) {
	b, n, _, _ := newTarget(t)
	if status, _, _, _ := n.send(t, alphaAddr, "/call?function=probe::echo", `{}`); status != 200 {
		t.Fatalf("shared: %d", status)
	}
	if err := b.Sync(context.Background(), nil); err != nil {
		t.Fatal(err)
	}
	if status, code, _, read := n.send(t, alphaAddr, "/call?function=probe::echo", `{}`); status != 403 || code != "FORBIDDEN" || read {
		t.Fatalf("unshared: %d %s", status, code)
	}
}

// A rename keeps the address, so it must not give a machine a second share of calls or connections.
func TestDirectCapsFollowTheAddress(t *testing.T) {
	b, n, _, p := newTarget(t)
	d := b.direct
	done := make(chan string, MaxCallsInFlight)
	for range MaxCallsInFlight {
		go func() {
			status, _, raw, _, err := n.try(alphaAddr, "POST", "/call?function=probe::wait", testVersion, `{}`)
			done <- fmt.Sprint(status, " ", string(raw), " ", err)
		}()
	}
	waitUntil(t, "64 calls in flight", func() bool { return d.running(alphaAddr) == MaxCallsInFlight })
	if status, code, _, read := n.send(t, alphaAddr, "/call?function=probe::echo", `{}`); status != 429 || code != "mesh_busy" || read {
		t.Fatalf("the 65th call: %d %s, body read %v", status, code, read)
	}
	n.set(func() { n.names[alphaAddr] = "renamed" })
	if status, code, _, _ := n.send(t, alphaAddr, "/call?function=probe::echo", `{}`); status != 429 || code != "mesh_busy" {
		t.Fatalf("the 65th call under another name: %d %s", status, code)
	}
	if status, _, _, _ := n.send(t, gammaAddr, "/call?function=probe::echo", `{}`); status != 200 {
		t.Fatalf("another machine while one is at its cap: %d", status)
	}
	// An unshared call takes no slot: only allowed calls count, as at the hub.
	if status, _, _, _ := n.send(t, alphaAddr, "/call?function=probe::other", `{}`); status != 403 {
		t.Fatalf("an unshared call at the cap: %d", status)
	}

	waitUntil(t, "the refused connections to close", func() bool { return d.open(alphaAddr) == MaxCallsInFlight })
	var idle []net.Conn
	defer func() {
		for _, c := range idle {
			c.Close()
		}
	}()
	for range maxDirectConns - MaxCallsInFlight {
		c, err := n.connect(alphaAddr)
		if err != nil {
			t.Fatal(err)
		}
		idle = append(idle, c)
	}
	waitUntil(t, "72 open connections", func() bool { return d.open(alphaAddr) == maxDirectConns })
	over, err := n.connect(alphaAddr)
	if err != nil {
		t.Fatal(err)
	}
	defer over.Close()
	_ = over.SetReadDeadline(time.Now().Add(3 * time.Second)) // well under the 10 s the server gives the headers
	if _, err = over.Read(make([]byte, 1)); err == nil || errors.Is(err, os.ErrDeadlineExceeded) {
		t.Fatalf("the 73rd connection of one address stays open: %v", err)
	}
	if status, _, _, _ := n.send(t, gammaAddr, "/call?function=probe::echo", `{}`); status != 200 || d.open(alphaAddr) != maxDirectConns {
		t.Fatalf("another machine while one holds its connections: %d, %d open", status, d.open(alphaAddr))
	}

	close(p.release)
	for range MaxCallsInFlight {
		if got := <-done; got != `200 "released" <nil>` {
			t.Fatalf("a waiting call: %s", got)
		}
	}
	waitUntil(t, "the count to return to zero", d.idle)
}

func TestDirectDeadlines(t *testing.T) {
	shorten(t, &directIO, 300*time.Millisecond)
	shorten(t, &directHead, 300*time.Millisecond)
	b, n, _, p := newTarget(t)
	d := b.direct

	// A connection that sends no call does not stay.
	silent, err := n.connect(gammaAddr)
	if err != nil {
		t.Fatal(err)
	}
	defer silent.Close()
	_ = silent.SetReadDeadline(time.Now().Add(5 * time.Second))
	if _, err = silent.Read(make([]byte, 1)); err == nil || errors.Is(err, os.ErrDeadlineExceeded) {
		t.Fatalf("a connection without a call stays open: %v", err)
	}
	// Nor does one whose headers do not end.
	// Only the status: the server closes on what it did not read, which may reset the connection after it.
	if status, _, _, _, _ := n.try(alphaAddr, "POST", "/call?function=probe::echo&pad="+strings.Repeat("a", 32<<10), testVersion, `{}`); status != http.StatusRequestHeaderFieldsTooLarge {
		t.Fatalf("32 KiB of headers: %d", status)
	}

	// Headers and a declared body that never comes: the slot must not wait for it.
	c, err := n.connect(alphaAddr)
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	start := time.Now()
	fmt.Fprintf(c, "POST /call?function=probe::echo HTTP/1.1\r\nHost: beta\r\nX-Mesh-Version: %s\r\nContent-Length: 5\r\n\r\n", testVersion)
	waitUntil(t, "the slot of the stalled upload", func() bool { return d.running(alphaAddr) == 1 })
	_ = c.SetReadDeadline(time.Now().Add(5 * time.Second))
	res, err := http.ReadResponse(bufio.NewReader(c), nil)
	if err != nil || res.StatusCode != 503 || time.Since(start) < directIO {
		t.Fatalf("a body that never comes: %v %v after %s", res, err, time.Since(start))
	}
	waitUntil(t, "the slot to return", d.idle)

	// A caller that stops reading a result larger than the socket buffers.
	p.big = json.RawMessage(`"` + strings.Repeat("a", 16<<20) + `"`) // no bigger: the target parses it for channel refs first, which takes seconds under -race
	c, err = n.connect(alphaAddr)
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	fmt.Fprintf(c, "POST /call?function=probe::big HTTP/1.1\r\nHost: beta\r\nX-Mesh-Version: %s\r\nContent-Length: 0\r\n\r\n", testVersion)
	waitUntil(t, "the slot of the big result", func() bool { return d.running(alphaAddr) == 1 })
	waitUntil(t, "the write to a caller that does not read to end", d.idle)

	// The read deadline is gone once the body is in: a call longer than it still answers.
	// Without a body the server is already watching the connection when the deadline is set.
	for _, body := range []string{"", `{}`} {
		if status, _, raw, _ := n.send(t, alphaAddr, "/call?function=probe::slow", body); status != 200 || string(raw) != `"slow"` {
			t.Fatalf("a call longer than the deadline, body %q: %d %s", body, status, raw)
		}
	}
}

// A handler that cannot bound its reads refuses: nothing may wait on a peer without a limit.
func TestDirectRefusesWithoutDeadlines(t *testing.T) {
	b, _, local, _ := newTarget(t)
	w := httptest.NewRecorder() // has no deadlines to set
	r := httptest.NewRequest("POST", "/call?function=probe::echo", strings.NewReader(`{}`))
	r.RemoteAddr = netip.AddrPortFrom(alphaAddr, 40000).String()
	r.Header.Set("X-Mesh-Version", testVersion)
	b.direct.serveCall(w, r)
	if w.Code != 500 || !strings.Contains(w.Body.String(), `"invocation_failed"`) || len(local.called("probe::echo")) != 0 {
		t.Fatalf("no deadline support: %d %s", w.Code, w.Body)
	}
}

// calling starts a call of beta's waiting function through alpha's mirror and returns once it runs on beta.
func calling(t *testing.T, alpha *fakeLocal, beta *Bridge) chan error {
	t.Helper()
	done := make(chan error, 1)
	go func() {
		_, err := alpha.mirrors["peer::beta::probe::wait"](context.Background(), json.RawMessage(`{}`))
		done <- err
	}()
	waitUntil(t, "the call to run on beta", func() bool { return beta.direct.running(alphaAddr) == 1 })
	return done
}

func within(t *testing.T, done chan error, what string) error {
	t.Helper()
	select {
	case err := <-done:
		return err
	case <-time.After(5 * time.Second):
		t.Fatalf("%s left the call waiting", what)
		return nil
	}
}

// These connections have no keepalive: only the map tells either end that the other is gone.
func TestDirectMapChangeEndsCalls(t *testing.T) {
	b, nb, _, p := newTarget(t)
	a, na, local := newCaller(t, nb, "")

	done := calling(t, local, b)
	a.MapChanged()
	b.MapChanged()
	p.let(t)
	if err := within(t, done, "a release"); err != nil || p.ends(t) != nil {
		t.Fatalf("a map change that kept both machines ended the call: %v", err)
	}

	for what, change := range map[string]func(){
		"the target leaving the map":         func() { delete(na.peers, "beta") },
		"the name moving to another address": func() { na.peers["beta"] = gammaAddr },
	} {
		done = calling(t, local, b)
		na.set(change)
		a.MapChanged()
		if err := within(t, done, what); codeOf(err) != "mesh_offline" || !strings.Contains(err.Error(), "no direct connection to beta") {
			t.Fatalf("%s: %v", what, err)
		}
		p.outlives(t, b.direct)
		na.set(func() { na.peers["beta"] = betaAddr })
	}

	done = calling(t, local, b)
	nb.set(func() { delete(nb.names, alphaAddr) })
	b.MapChanged()
	if err := within(t, done, "the target closing the connection"); codeOf(err) != "mesh_offline" {
		t.Fatalf("the caller of a closed connection: %v", err)
	}
	p.outlives(t, b.direct)
	waitUntil(t, "beta's connection", func() bool { return b.direct.open(alphaAddr) == 0 })
}

// outlives checks that alpha's call on beta, whose connection is gone, holds its
// slot until the function returns: the engine runs it to its end anyway.
func (p *probe) outlives(t *testing.T, d *direct) {
	t.Helper()
	if n := d.running(alphaAddr); n != 1 {
		t.Fatalf("alpha's slots on beta while its function runs: %d", n)
	}
	p.let(t)
	if err := p.ends(t); err != nil {
		t.Fatalf("the function on beta was ended with its connection: %v", err)
	}
	waitUntil(t, "beta's slot", d.idle)
}

func TestDirectOffEndsCalls(t *testing.T) {
	b, nb, _, p := newTarget(t)
	a, na, local := newCaller(t, nb, "")

	done := calling(t, local, b)
	d := a.direct
	if err := a.SetDirect(nil, ""); err != nil {
		t.Fatal(err)
	}
	if err := within(t, done, "turning the caller off"); codeOf(err) != "mesh_offline" {
		t.Fatalf("a call in flight when its machine turned peer to peer off: %v", err)
	}
	p.outlives(t, b.direct)
	// A mirror that read the path just before it closed.
	dials := na.dials()
	if _, err := d.call(context.Background(), "beta", betaAddr, "probe::echo", json.RawMessage(`{}`)); codeOf(err) != "mesh_offline" || na.dials() != dials {
		t.Fatalf("a call on a closed path: %v, %d dials", err, na.dials()-dials)
	}
	if err := a.SetDirect(na, testVersion); err != nil {
		t.Fatal(err)
	}

	done = calling(t, local, b)
	d = b.direct
	if err := b.SetDirect(nil, ""); err != nil {
		t.Fatal(err)
	}
	if err := within(t, done, "turning the target off"); codeOf(err) != "mesh_offline" {
		t.Fatalf("the caller of a target that turned off: %v", err)
	}
	p.outlives(t, d)
	if _, err := local.mirrors["peer::beta::probe::echo"](context.Background(), json.RawMessage(`{}`)); codeOf(err) != "mesh_offline" {
		t.Fatalf("a call to a closed listener: %v", err)
	}
	// Nothing is reused: the next call reaches the new listener.
	if err := b.SetDirect(nb, testVersion); err != nil {
		t.Fatal(err)
	}
	if out, err := local.mirrors["peer::beta::probe::echo"](context.Background(), json.RawMessage(`{"x":1}`)); err != nil || string(out.(json.RawMessage)) != `{"x":1}` {
		t.Fatalf("a call after the target reopened: %s %v", out, err)
	}
}

// SetDirect owns the order: listen before asking to be shown, close before withdrawing.
func TestSetDirectOrder(t *testing.T) {
	b := NewBridge(engine(&[]map[string]any{}), "local")
	reachable := func(n *fakeNet) bool {
		c, err := net.Dial("tcp", n.listen)
		if err == nil {
			c.Close()
		}
		return err == nil
	}
	n := &fakeNet{}
	if err := b.SetDirect(n, testVersion); err != nil || !slices.Equal(n.did(), []string{"listen", "ask"}) || b.direct == nil || !reachable(n) {
		t.Fatalf("on: %v %v", n.did(), err)
	}
	if err := b.SetDirect(nil, ""); err != nil || !slices.Equal(n.did(), []string{"listen", "ask", "close", "withdraw"}) || b.direct != nil || reachable(n) {
		t.Fatalf("off: %v %v", n.did(), err)
	}
	if err := b.SetDirect(nil, ""); err != nil || len(n.did()) != 4 {
		t.Fatalf("off twice: %v %v", n.did(), err)
	}

	n = &fakeNet{failListen: errors.New("port is in use")}
	if err := b.SetDirect(n, testVersion); err == nil || !slices.Equal(n.did(), []string{"listen"}) || b.direct != nil {
		t.Fatalf("a failed listen: %v %v", n.did(), err)
	}
	n = &fakeNet{failAsk: errors.New("refused")}
	if err := b.SetDirect(n, testVersion); err == nil || !slices.Equal(n.did(), []string{"listen", "ask", "close"}) || b.direct != nil || reachable(n) {
		t.Fatalf("a failed ask: %v %v", n.did(), err)
	}
	n = &fakeNet{failWithdraw: errors.New("refused")}
	if err := b.SetDirect(n, testVersion); err != nil {
		t.Fatal(err)
	}
	if err := b.SetDirect(nil, ""); err == nil || !slices.Equal(n.did(), []string{"listen", "ask", "close", "withdraw"}) || b.direct != nil || reachable(n) {
		t.Fatalf("a failed withdrawal still closes: %v %v", n.did(), err)
	}

	n, next := &fakeNet{}, &fakeNet{}
	if err := b.SetDirect(n, testVersion); err != nil {
		t.Fatal(err)
	}
	if err := b.SetDirect(next, testVersion); err != nil || !slices.Equal(n.did(), []string{"listen", "ask", "close", "withdraw"}) || !slices.Equal(next.did(), []string{"listen", "ask"}) {
		t.Fatalf("on over on: %v then %v, %v", n.did(), next.did(), err)
	}
	b.Close()
	if !slices.Equal(next.did(), []string{"listen", "ask", "close", "withdraw"}) || reachable(next) {
		t.Fatalf("Close: %v", next.did())
	}
}

// The caller takes an answer only from the target: it follows no redirect, reuses no connection and reads no more than the cap.
func TestDirectCallerTakesOnlyTheTargetsAnswer(t *testing.T) {
	_, na, local := newCaller(t, nil, "")
	var mu sync.Mutex
	var got []*http.Request
	var bodies []string
	answer := func(w http.ResponseWriter, _ *http.Request) { _, _ = w.Write([]byte(`{"ok":true}`)) }
	stand := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		mu.Lock()
		got, bodies = append(got, r), append(bodies, string(body))
		f := answer
		mu.Unlock()
		f(w, r)
	}))
	defer stand.Close()
	na.set(func() { na.plain = stand.Listener.Addr().String() })
	echo := func(payload string) (any, error) {
		return local.mirrors["peer::beta::probe::echo"](context.Background(), json.RawMessage(payload))
	}
	answers := func(f func(http.ResponseWriter, *http.Request)) {
		mu.Lock()
		defer mu.Unlock()
		answer = f
	}
	seen := func() int {
		mu.Lock()
		defer mu.Unlock()
		return len(got)
	}

	out, err := echo(`{"x":1}`)
	if err != nil || string(out.(json.RawMessage)) != `{"ok":true}` {
		t.Fatalf("result %s %v", out, err)
	}
	r := got[0]
	if r.Method != "POST" || r.URL.Path != "/call" || r.URL.Query().Get("function") != "probe::echo" || r.Header.Get("X-Mesh-Version") != testVersion || bodies[0] != `{"x":1}` ||
		!r.Close || r.Header.Get("Expect") != "100-continue" || !slices.Equal(na.dialed, []string{"[2001:db8::b]:9136"}) {
		t.Fatalf("request %s %s %v, body %s, dialed %v", r.Method, r.URL, r.Header, bodies[0], na.dialed)
	}
	// The stand-in keeps connections alive, so a second dial is the caller's doing.
	if _, err = echo(`{}`); err != nil || na.dials() != 2 {
		t.Fatalf("two calls dialed %d times: %v", na.dials(), err)
	}

	answers(func(w http.ResponseWriter, r *http.Request) {
		http.Redirect(w, r, "/elsewhere", http.StatusTemporaryRedirect)
	})
	if _, err = echo(`{"secret":1}`); codeOf(err) != "mesh_offline" || na.dials() != 3 || seen() != 3 {
		t.Fatalf("a redirect: %v, %d dials, %d requests", err, na.dials(), seen())
	}
	answers(func(w http.ResponseWriter, _ *http.Request) { http.Error(w, "bad gateway", http.StatusBadGateway) })
	if _, err = echo(`{}`); codeOf(err) != "mesh_offline" {
		t.Fatalf("an answer that is not the target's: %v", err)
	}
	answers(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusForbidden)
		_, _ = w.Write([]byte(`{"code":"FORBIDDEN","message":"function is not shared with this machine"}`))
	})
	var ie *iii.InvocationError
	if _, err = echo(`{}`); !errors.As(err, &ie) || ie.Code != "FORBIDDEN" || ie.Message != "function is not shared with this machine" {
		t.Fatalf("the target's refusal: %v", err)
	}
	answers(func(http.ResponseWriter, *http.Request) {})
	if out, err = echo(`{}`); out != nil || err != nil {
		t.Fatalf("no result: %v %v", out, err) // an empty RawMessage would not encode
	}

	// Channel refs neither leave nor enter this machine, whatever the target does, as the hub refuses both.
	answers(func(w http.ResponseWriter, _ *http.Request) { _, _ = w.Write([]byte(channelRef)) })
	if _, err = echo(`{}`); codeOf(err) != "channels_unsupported" {
		t.Fatalf("a result with a channel ref: %v", err)
	}
	sent := seen()
	if _, err = echo(channelRef); codeOf(err) != "channels_unsupported" || seen() != sent {
		t.Fatalf("a payload with a channel ref: %v, %d requests", err, seen()-sent)
	}

	shorten(t, &maxDirectBody, 64)
	answers(func(w http.ResponseWriter, _ *http.Request) { _, _ = w.Write(bytes.Repeat([]byte("1"), 65)) })
	if _, err = echo(`{}`); codeOf(err) != "payload_too_large" {
		t.Fatalf("a result over the cap: %v", err)
	}
	// It stops reading at the cap, whatever the target goes on sending.
	cut := make(chan bool, 1)
	answers(func(w http.ResponseWriter, _ *http.Request) {
		for range 512 { // 32 MiB, more than the sockets buffer
			if _, err := w.Write(bytes.Repeat([]byte("1"), 64<<10)); err != nil {
				cut <- true
				return
			}
		}
		cut <- false
	})
	if _, err = echo(`{}`); codeOf(err) != "payload_too_large" || !<-cut {
		t.Fatalf("a result without an end was read to its end: %v", err)
	}
	answers(func(w http.ResponseWriter, _ *http.Request) { _, _ = w.Write(bytes.Repeat([]byte("1"), 64)) })
	if out, err = echo(`{}`); err != nil || len(out.(json.RawMessage)) != 64 {
		t.Fatalf("a result of exactly the cap: %v", err)
	}
	dials := na.dials()
	if _, err = echo(`"` + strings.Repeat("a", 63) + `"`); codeOf(err) != "payload_too_large" || na.dials() != dials {
		t.Fatalf("a payload over the cap: %v, %d dials", err, na.dials()-dials)
	}
}

// The target closes after one answer, whatever the caller asks for.
func TestDirectAnswersOneCallPerConnection(t *testing.T) {
	_, n, _, _ := newTarget(t)
	c, err := n.connect(alphaAddr)
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	_ = c.SetDeadline(time.Now().Add(5 * time.Second))
	request := fmt.Sprintf("POST /call?function=probe::echo HTTP/1.1\r\nHost: beta\r\nX-Mesh-Version: %s\r\nContent-Length: 2\r\n\r\n{}", testVersion)
	br := bufio.NewReader(c)
	_, _ = io.WriteString(c, request)
	res, err := http.ReadResponse(br, nil)
	if err != nil || res.StatusCode != 200 || !res.Close {
		t.Fatalf("first call: %v %v", res, err)
	}
	_, _ = io.Copy(io.Discard, res.Body)
	_, _ = io.WriteString(c, request)
	if res, err = http.ReadResponse(br, nil); err == nil {
		t.Fatalf("a second call on the same connection was answered: %d", res.StatusCode)
	}
}

type lockedLog struct {
	mu sync.Mutex
	b  bytes.Buffer
}

func (l *lockedLog) Write(p []byte) (int, error) {
	l.mu.Lock()
	defer l.mu.Unlock()
	return l.b.Write(p)
}

func (l *lockedLog) refusals() []string {
	l.mu.Lock()
	defer l.mu.Unlock()
	var out []string
	for line := range strings.Lines(l.b.String()) {
		if strings.Contains(line, "mesh direct: refused") {
			out = append(out, line)
		}
	}
	return out
}

// The caller sets the request rate, so it must not set the log rate or the size of a line.
func TestDirectRefusalLogIsLimited(t *testing.T) {
	var out lockedLog
	log.SetOutput(&out)
	t.Cleanup(func() { log.SetOutput(os.Stderr) })
	shorten(t, &refusalEvery, time.Hour)
	b, n, _, _ := newTarget(t)
	hostile := "/call?function=probe::x%0Aforged+line+" + strings.Repeat("y", 300)
	for range 100 {
		if status, _, _, _ := n.send(t, alphaAddr, hostile, `{"payload":"never logged"}`); status != 403 {
			t.Fatalf("refusal: %d", status)
		}
	}
	lines := out.refusals()
	if len(lines) != 1 || !strings.Contains(lines[0], `for alpha (busy false), 0 refusals since the last line`) || !strings.Contains(lines[0], `probe::x\nforged`) ||
		strings.Contains(lines[0], strings.Repeat("y", 200)) || strings.Contains(lines[0], "never logged") {
		t.Fatalf("100 refusals from one address logged %d lines: %q", len(lines), lines)
	}
	n.send(t, unknownAddr, "/call?function=probe::echo", `{}`) // another address has its own line, by address when the map has no name
	if lines = out.refusals(); len(lines) != 2 || !strings.Contains(lines[1], "for 2001:db8::d (busy false), 0 refusals") {
		t.Fatalf("another address: %q", lines)
	}
	b.direct.mu.Lock()
	b.direct.refused[alphaAddr].next = time.Now() // the window ends
	b.direct.mu.Unlock()
	n.send(t, alphaAddr, "/call?function=probe::other", `{}`)
	if lines = out.refusals(); len(lines) != 3 || !strings.Contains(lines[2], `refused "probe::other" for alpha (busy false), 99 refusals since the last line`) {
		t.Fatalf("after the window: %q", lines)
	}
	b.direct.mu.Lock()
	b.direct.refused[alphaAddr].next = time.Now()
	b.direct.mu.Unlock()
	n.send(t, alphaAddr, "/call?function=probe::other", `{}`)
	if lines = out.refusals(); len(lines) != 4 || !strings.Contains(lines[3], ", 0 refusals since the last line") {
		t.Fatalf("the count starts again with each line: %q", lines)
	}
}

func TestMirrorRouting(t *testing.T) {
	url, hub := meshHub(t, false)
	b, nb, betaLocal, _ := newTarget(t)
	a, na, local := newCaller(t, nb, url)
	waitUntil(t, "connected", func() bool {
		a.mu.Lock()
		defer a.mu.Unlock()
		return a.online != nil && a.online.Err() == nil
	})
	echo := func() (string, error) {
		out, err := local.mirrors["peer::beta::probe::echo"](context.Background(), json.RawMessage(`{"x":1}`))
		raw, _ := out.(json.RawMessage)
		return string(raw), err
	}
	throughHub := func(what, owner string) {
		t.Helper()
		select {
		case f := <-hub:
			if !strings.Contains(string(f["data"]), `"owner":"`+owner+`"`) {
				t.Fatalf("%s: the hub got %s", what, f["data"])
			}
		default:
			t.Fatalf("%s: no mesh::call reached the hub", what)
		}
	}
	direct := func(what string) {
		t.Helper()
		if len(hub) != 0 {
			t.Fatalf("%s: a mesh::call reached the hub", what)
		}
	}

	if out, err := echo(); err != nil || out != `{"x":1}` || na.dials() != 1 || len(betaLocal.called("probe::echo")) != 1 {
		t.Fatalf("both on: %s %v, %d dials", out, err, na.dials())
	}
	direct("both on")
	// The local id travels: the target knows its functions without its own name in front.
	if na.dialed[0] != "[2001:db8::b]:9136" {
		t.Fatalf("dialed %v", na.dialed)
	}

	if _, err := local.mirrors["hub::shell::exec"](context.Background(), json.RawMessage(`{}`)); err != nil || na.dials() != 1 {
		t.Fatalf("the hub's function: %v, %d dials", err, na.dials())
	}
	throughHub("owner hub, although the hub is in the map", "hub")

	if err := b.Sync(context.Background(), nil); err != nil {
		t.Fatal(err)
	}
	if _, err := echo(); codeOf(err) != "FORBIDDEN" {
		t.Fatalf("the target's refusal: %v", err)
	}
	direct("a refusal by the target")
	// A payload the target never reads must not cost the caller the refusal: a close on unread data resets the connection.
	large := json.RawMessage(`"` + strings.Repeat("a", 1<<20) + `"`)
	for range 40 {
		if _, err := local.mirrors["peer::beta::probe::echo"](context.Background(), large); codeOf(err) != "FORBIDDEN" {
			t.Fatalf("the refusal of a call with a large payload: %v", err)
		}
	}
	if err := b.Sync(context.Background(), []string{"probe"}); err != nil {
		t.Fatal(err)
	}

	na.set(func() { na.heard = time.Now().Add(-directGrace - time.Minute) })
	dials := na.dials()
	if _, err := echo(); codeOf(err) != "mesh_offline" || na.dials() != dials {
		t.Fatalf("a hub not heard for longer than the grace: %v, %d dials", err, na.dials()-dials)
	}
	direct("a hub not heard")
	na.set(func() { na.heard = time.Now() })

	na.set(func() { na.to = nil })
	if _, err := echo(); codeOf(err) != "mesh_offline" || !strings.Contains(err.Error(), "no direct connection to beta") {
		t.Fatalf("a failed dial: %v", err)
	}
	direct("a failed dial")
	na.set(func() { na.to = nb })

	na.set(func() { delete(na.peers, "beta") })
	dials = na.dials()
	if out, err := echo(); err != nil || !strings.Contains(out, `"owner":"beta"`) || na.dials() != dials {
		t.Fatalf("a machine that is not in the map: %s %v", out, err)
	}
	throughHub("a machine that is not in the map", "beta")
	na.set(func() { na.peers["beta"] = betaAddr })

	if err := a.SetDirect(nil, ""); err != nil {
		t.Fatal(err)
	}
	if _, err := echo(); err != nil || na.dials() != dials {
		t.Fatalf("this machine off: %v", err)
	}
	throughHub("this machine off", "beta")
}

func TestDirectCallNeedsNoHubConnection(t *testing.T) {
	_, nb, _, _ := newTarget(t)
	_, _, local := newCaller(t, nb, "")
	if _, err := local.mirrors["hub::shell::exec"](context.Background(), json.RawMessage(`{}`)); codeOf(err) != "mesh_offline" {
		t.Fatalf("the hub's function with the hub connection down: %v", err)
	}
	if out, err := local.mirrors["peer::beta::probe::echo"](context.Background(), json.RawMessage(`{"x":1}`)); err != nil || string(out.(json.RawMessage)) != `{"x":1}` {
		t.Fatalf("a direct call with the hub connection down: %s %v", out, err)
	}
}

// The page learns the path a call opened and, with no event for it, where that path went next; a busy caller must not recompute the status per call.
func TestDirectCallRefreshesStatusAtMostEveryFiveSeconds(t *testing.T) {
	shorten(t, &statusEvery, 200*time.Millisecond)
	_, nb, _, _ := newTarget(t)
	a, _, local := newCaller(t, nb, "")
	var changes atomic.Int32
	a.mu.Lock()
	a.Changed = func() { changes.Add(1) }
	a.mu.Unlock()
	echo := func() {
		t.Helper()
		if _, err := local.mirrors["peer::beta::probe::echo"](context.Background(), json.RawMessage(`{}`)); err != nil {
			t.Fatal(err)
		}
	}
	echo()
	echo()
	if changes.Load() != 1 {
		t.Fatalf("two calls marked the status %d times", changes.Load())
	}
	waitUntil(t, "the one more look after the calls", func() bool { return changes.Load() == 2 })
	time.Sleep(3 * statusEvery)
	if changes.Load() != 2 {
		t.Fatalf("with no call, the status was marked %d times, want no more looks", changes.Load())
	}
	echo()
	if changes.Load() != 3 {
		t.Fatalf("a call after the look marked the status %d times", changes.Load())
	}
}

func TestRemoteReportsThePath(t *testing.T) {
	b := NewBridge(engine(&[]map[string]any{}), "local")
	defer b.Close()
	catalog := node.Catalog{Rev: "r1", Functions: []node.CatalogEntry{{ID: "shell::exec", Owner: "hub"}}}
	for _, m := range []string{"m-direct", "m-relay", "m-idle", "m-off"} {
		catalog.Functions = append(catalog.Functions, node.CatalogEntry{ID: m + "::probe::echo", Owner: m})
	}
	b.SetCatalog(catalog)
	paths := func() []string {
		var out []string
		for _, r := range b.Remote() {
			out = append(out, r.Machine+"="+r.Path)
		}
		return out
	}
	if got := paths(); !slices.Equal(got, []string{"hub=", "m-direct=hub", "m-idle=hub", "m-off=hub", "m-relay=hub"}) {
		t.Fatalf("peer to peer off: %v", got)
	}
	n := &fakeNet{paths: []meshnet.PeerStatus{{Name: "hub", Path: "relay hub"}, {Name: "m-direct", Path: "direct 192.0.2.1:41641"}, {Name: "m-idle", Path: "idle"}, {Name: "m-relay", Path: "relay hub"}}}
	if err := b.SetDirect(n, testVersion); err != nil {
		t.Fatal(err)
	}
	if got := paths(); !slices.Equal(got, []string{"hub=", "m-direct=direct", "m-idle=idle", "m-off=hub", "m-relay=relay"}) {
		t.Fatalf("peer to peer on: %v", got)
	}
	if raw, _ := json.Marshal(b.Remote()[:2]); string(raw) != `[{"machine":"hub","functions":["hub::shell::exec"]},{"machine":"m-direct","functions":["peer::m-direct::probe::echo"],"path":"direct"}]` {
		t.Fatalf("status %s", raw)
	}
}
