package gateway

import (
	"context"
	"fmt"
	"net"
	"net/http"
	"net/http/httptest"
	"net/netip"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/coder/websocket"
)

type received struct {
	values []string
	ticket string
}

func proxyServer(t *testing.T, seen chan received) *httptest.Server {
	t.Helper()
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		values := r.Header.Values(TicketHeader)
		seen <- received{values, r.Header.Get(TicketHeader)}
		c, err := websocket.Accept(w, r, &websocket.AcceptOptions{InsecureSkipVerify: true})
		if err != nil {
			return
		}
		defer c.CloseNow()
		for {
			kind, b, err := c.Read(r.Context())
			if err != nil {
				return
			}
			if c.Write(r.Context(), kind, b) != nil {
				return
			}
		}
	}))
	t.Cleanup(s.Close)
	return s
}
func testGateway(t *testing.T, lookup func(netip.Addr) (string, bool)) (*Gateway, *Tickets, string, chan received) {
	t.Helper()
	seen := make(chan received, 10)
	proxy := proxyServer(t, seen)
	tickets := NewTickets(10*time.Second, time.Now)
	g := New(lookup, "ws"+strings.TrimPrefix(proxy.URL, "http"), tickets)
	g.SetActive(true)
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	go g.Serve(ln)
	t.Cleanup(func() { g.Close(); ln.Close() })
	return g, tickets, "ws://" + ln.Addr().String(), seen
}
func known(netip.Addr) (string, bool) { return "laptop", true }
func dial(t *testing.T, url string, header http.Header) *websocket.Conn {
	t.Helper()
	c, _, err := websocket.Dial(t.Context(), url, &websocket.DialOptions{HTTPHeader: header})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { c.CloseNow() })
	return c
}
func echo(t *testing.T, c *websocket.Conn) {
	t.Helper()
	ctx, cancel := context.WithTimeout(t.Context(), time.Second)
	defer cancel()
	if err := c.Write(ctx, websocket.MessageText, []byte("hello")); err != nil {
		t.Fatal(err)
	}
	_, b, err := c.Read(ctx)
	if err != nil || string(b) != "hello" {
		t.Fatalf("echo %q %v", b, err)
	}
}
func closed(t *testing.T, c *websocket.Conn) {
	t.Helper()
	ctx, cancel := context.WithTimeout(t.Context(), time.Second)
	defer cancel()
	if _, _, err := c.Read(ctx); err == nil || ctx.Err() != nil {
		t.Fatalf("connection not promptly closed: %v", err)
	}
}
func TestGatewayStripsClientTicketHeader(t *testing.T) {
	_, tickets, url, seen := testGateway(t, known)
	c := dial(t, url, http.Header{"X-Iii-Mesh-Ticket": {"forged", "also-forged"}})
	echo(t, c)
	request := <-seen
	if len(request.values) != 1 {
		t.Fatalf("ticket headers: %v", request.values)
	}
	if node, ok := tickets.Consume(request.ticket); !ok || node != "laptop" {
		t.Fatalf("ticket: %q %v", node, ok)
	}
}
func TestGatewayRefusesUnknownPeer(t *testing.T) {
	_, _, url, seen := testGateway(t, func(netip.Addr) (string, bool) { return "", false })
	c, res, err := websocket.Dial(t.Context(), url, nil)
	if c != nil {
		c.CloseNow()
	}
	if err == nil || res.StatusCode != http.StatusForbidden {
		t.Fatalf("unknown peer: %v %v", res, err)
	}
	select {
	case <-seen:
		t.Fatal("unknown peer reached proxy")
	default:
	}
}
func TestCloseNodeDropsConnections(t *testing.T) {
	g, tickets, url, seen := testGateway(t, known)
	a := dial(t, url, nil)
	ra := <-seen
	b := dial(t, url, nil)
	rb := <-seen
	_, oa, ok := tickets.ConsumeOwned(ra.ticket)
	if !ok {
		t.Fatal("missing owner")
	}
	_, ob, ok := tickets.ConsumeOwned(rb.ticket)
	if !ok {
		t.Fatal("missing owner")
	}
	done := make(chan struct{})
	oa.OnClose(func() { close(done) })
	a.CloseNow()
	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatal("disconnect did not revoke owner")
	}
	late := false
	if oa.OnClose(func() { late = true }) || !late {
		t.Fatal("close-before-auth callback missed")
	}
	echo(t, b)
	sibling := make(chan struct{})
	ob.OnClose(func() { close(sibling) })
	g.CloseNode("other")
	echo(t, b)
	g.CloseNode("laptop")
	closed(t, b)
	select {
	case <-sibling:
	case <-time.After(time.Second):
		t.Fatal("node close missed owner")
	}
}
func TestGatewayActivationAndShutdown(t *testing.T) {
	g, tickets, url, seen := testGateway(t, known)
	c := dial(t, url, nil)
	r := <-seen
	_, owner, ok := tickets.ConsumeOwned(r.ticket)
	if !ok {
		t.Fatal("missing owner")
	}
	done := make(chan struct{})
	owner.OnClose(func() { close(done) })
	g.SetActive(false)
	closed(t, c)
	<-done
	_, res, err := websocket.Dial(t.Context(), url, nil)
	if err == nil || res.StatusCode != 503 {
		t.Fatalf("inactive gateway: %v %v", res, err)
	}
	g.SetActive(true)
	c = dial(t, url, nil)
	r = <-seen
	g.Close()
	closed(t, c)
	if _, _, ok := tickets.ConsumeOwned(r.ticket); ok {
		t.Fatal("closed ticket accepted")
	}
	g.SetActive(true)
	_, res, err = websocket.Dial(t.Context(), url, nil)
	if err == nil || (res != nil && res.StatusCode != 503) {
		t.Fatalf("closed gateway reactivated: %v %v", res, err)
	}
}
func TestFailedUpgradeClosesOwner(t *testing.T) {
	seen := make(chan received, 1)
	proxy := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		seen <- received{nil, r.Header.Get(TicketHeader)}
		http.Error(w, "refused", 503)
	}))
	defer proxy.Close()
	tickets := NewTickets(10*time.Second, time.Now)
	g := New(known, proxy.URL, tickets)
	g.SetActive(true)
	defer g.Close()
	s := httptest.NewServer(g)
	defer s.Close()
	res, err := http.Get(s.URL)
	if err != nil {
		t.Fatal(err)
	}
	res.Body.Close()
	r := <-seen
	if _, _, ok := tickets.ConsumeOwned(r.ticket); ok {
		t.Fatal("failed handshake ticket still live")
	}
}

func TestCloseNodeDuringUpstreamAuthentication(t *testing.T) {
	tickets := NewTickets(10*time.Second, time.Now)
	authenticated := make(chan *Connection, 1)
	proxy := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, owner, ok := tickets.ConsumeOwned(r.Header.Get(TicketHeader))
		if !ok {
			http.Error(w, "no ticket", 403)
			return
		}
		authenticated <- owner
		<-r.Context().Done()
	}))
	defer proxy.Close()
	g := New(known, proxy.URL, tickets)
	g.SetActive(true)
	defer g.Close()
	s := httptest.NewServer(g)
	defer s.Close()
	requestDone := make(chan struct{})
	go func() {
		defer close(requestDone)
		res, err := http.Get(s.URL)
		if err == nil {
			res.Body.Close()
		}
	}()
	var owner *Connection
	select {
	case owner = <-authenticated:
	case <-time.After(time.Second):
		t.Fatal("upstream did not authenticate")
	}
	revoked := make(chan struct{})
	owner.OnClose(func() { g.CloseNode("other"); v := tickets.Mint("other"); tickets.Consume(v); close(revoked) })
	g.CloseNode("laptop")
	select {
	case <-revoked:
	case <-time.After(time.Second):
		t.Fatal("callback deadlocked")
	}
	select {
	case <-requestDone:
	case <-time.After(time.Second):
		t.Fatal("pending upstream survived close")
	}
}

func TestGatewayClosesOwnerOnUpstreamDisconnect(t *testing.T) {
	tickets := NewTickets(10*time.Second, time.Now)
	ownerReady := make(chan *Connection, 1)
	disconnect := make(chan struct{})
	proxy := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, owner, ok := tickets.ConsumeOwned(r.Header.Get(TicketHeader))
		if !ok {
			http.Error(w, "no ticket", 403)
			return
		}
		c, err := websocket.Accept(w, r, &websocket.AcceptOptions{InsecureSkipVerify: true})
		if err != nil {
			return
		}
		defer c.CloseNow()
		ownerReady <- owner
		<-disconnect
	}))
	defer proxy.Close()
	g := New(known, proxy.URL, tickets)
	g.SetActive(true)
	defer g.Close()
	s := httptest.NewServer(g)
	defer s.Close()
	c := dial(t, "ws"+strings.TrimPrefix(s.URL, "http"), nil)
	owner := <-ownerReady
	revoked := make(chan struct{})
	owner.OnClose(func() { close(revoked) })
	close(disconnect)
	closed(t, c)
	select {
	case <-revoked:
	case <-time.After(time.Second):
		t.Fatal("upstream disconnect did not revoke owner")
	}
}

func TestGatewayReportsFailureAndRecoversWithoutDroppingSiblings(t *testing.T) {
	for _, mode := range []int32{1, 2} {
		t.Run(fmt.Sprint(mode), func(t *testing.T) {
			var failure atomic.Int32
			seen := make(chan string, 8)
			proxy := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				seen <- r.Header.Get(TicketHeader)
				switch failure.Load() {
				case 1:
					http.Error(w, "refused", 503)
					return
				case 2:
					conn, _, err := w.(http.Hijacker).Hijack()
					if err == nil {
						conn.Close()
					}
					return
				}
				c, err := websocket.Accept(w, r, &websocket.AcceptOptions{InsecureSkipVerify: true})
				if err != nil {
					return
				}
				defer c.CloseNow()
				for {
					kind, b, err := c.Read(r.Context())
					if err != nil {
						return
					}
					if c.Write(r.Context(), kind, b) != nil {
						return
					}
				}
			}))
			defer proxy.Close()
			tickets := NewTickets(10*time.Second, time.Now)
			g := New(known, proxy.URL, tickets)
			g.SetActive(true)
			defer g.Close()
			s := httptest.NewServer(g)
			defer s.Close()
			url := "ws" + strings.TrimPrefix(s.URL, "http")
			sibling := dial(t, url, nil)
			<-seen
			failure.Store(mode)
			c, _, err := websocket.Dial(t.Context(), url, nil)
			if err == nil {
				c.CloseNow()
				t.Fatal("failed upstream accepted")
			}
			failedTicket := <-seen
			if g.Status() == "ok" {
				t.Fatal("failed proxy reported healthy")
			}
			if _, _, ok := tickets.ConsumeOwned(failedTicket); ok {
				t.Fatal("failure left live ticket")
			}
			echo(t, sibling)
			failure.Store(0)
			retry := dial(t, url, nil)
			<-seen
			echo(t, retry)
			if g.Status() != "ok" {
				t.Fatalf("successful retry did not restore health: %s", g.Status())
			}
		})
	}
}

func TestGatewayReportsInvalidProxyURL(t *testing.T) {
	for _, target := range []string{"://", "ftp://127.0.0.1"} {
		t.Run(target, func(t *testing.T) {
			g := New(known, target, NewTickets(10*time.Second, time.Now))
			g.SetActive(true)
			defer g.Close()
			request := httptest.NewRequest(http.MethodGet, "http://mesh/", nil)
			request.RemoteAddr = "127.0.0.1:1234"
			response := httptest.NewRecorder()
			g.ServeHTTP(response, request)
			if response.Code != 502 {
				t.Fatalf("invalid URL status: %d", response.Code)
			}
			if g.Status() == "ok" {
				t.Fatal("invalid upstream reported healthy")
			}
		})
	}
}

func TestGatewayRejectsRemovalDuringPeerLookup(t *testing.T) {
	seen := make(chan received, 1)
	proxy := proxyServer(t, seen)
	entered, resume := make(chan struct{}), make(chan struct{})
	var authorized, firstLookup atomic.Bool
	authorized.Store(true)
	firstLookup.Store(true)
	g := New(func(netip.Addr) (string, bool) {
		allowed := authorized.Load()
		if firstLookup.Swap(false) {
			close(entered)
			<-resume
		}
		return "laptop", allowed
	}, proxy.URL, NewTickets(10*time.Second, time.Now))
	g.SetActive(true)
	defer g.Close()
	s := httptest.NewServer(g)
	defer s.Close()
	result := make(chan error, 1)
	go func() {
		c, _, err := websocket.Dial(t.Context(), "ws"+strings.TrimPrefix(s.URL, "http"), nil)
		if c != nil {
			c.CloseNow()
		}
		result <- err
	}()
	<-entered
	authorized.Store(false)
	g.CloseNode("laptop")
	close(resume)
	if err := <-result; err == nil {
		t.Fatal("removed node completed upgrade after revocation finished using stale lookup")
	}
	select {
	case <-seen:
		t.Fatal("revoked admission reached upstream")
	default:
	}
	authorized.Store(true)
	retry := dial(t, "ws"+strings.TrimPrefix(s.URL, "http"), nil)
	echo(t, retry)
	<-seen
}
