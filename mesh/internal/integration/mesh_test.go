package integration

import (
	"context"
	"crypto/subtle"
	"errors"
	"fmt"
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

	"github.com/iii-hq/workers/mesh/internal/coordinator"
	"github.com/iii-hq/workers/mesh/internal/join"
	"github.com/iii-hq/workers/mesh/internal/meshnet"
	"github.com/iii-hq/workers/mesh/internal/relay"
	"tailscale.com/derp/derpserver"
	"tailscale.com/ipn/ipnstate"
	"tailscale.com/tsnet"
)

type hub struct {
	dir, address string
	redemptions  atomic.Int32
	run          *hubRun
}

type hubRun struct {
	c         *coordinator.Coordinator
	front     *httptest.Server
	admission *httptest.Server
	derp      *derpserver.Server
	shim      *meshnet.Shim
	listener  net.Listener
	once      sync.Once
}

func (r *hubRun) close() {
	r.once.Do(func() {
		r.c.Close()
		if r.shim != nil {
			r.shim.Close()
		}
		r.derp.Close()
		r.front.Close()
		r.admission.Close()
	})
}

func newHub(t *testing.T) *hub {
	t.Helper()
	t.Setenv("TS_NO_LOGS_NO_SUPPORT", "true")
	h := &hub{dir: t.TempDir(), address: "127.0.0.1:0"}
	ctx, cancel := context.WithTimeout(t.Context(), time.Minute)
	t.Cleanup(cancel)
	h.start(t, ctx)
	return h
}

func (h *hub) start(t *testing.T, ctx context.Context) {
	t.Helper()
	var c *coordinator.Coordinator
	c, err := coordinator.New(h.dir, func(secret, addr string) (string, error) {
		if subtle.ConstantTimeCompare([]byte(secret), []byte(join.Secret(c.Identity(), c.Code()))) != 1 {
			return "", join.ErrCode
		}
		return "", nil
	})
	if err != nil {
		t.Fatal(err)
	}
	admission := httptest.NewServer(relay.AdmitHandler(c.Known))
	derp := relay.NewWithKey(c.RelayKey(), admission.URL)
	mux := http.NewServeMux()
	c.Handle(mux)
	relay.Handle(mux, derp)
	joinHandler := join.Handler(c.Identity(), c.Code, join.NewLimit(10, 10*time.Minute, time.Now), nil)
	mux.Handle("/join", http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		h.redemptions.Add(1)
		joinHandler.ServeHTTP(w, r)
	}))
	listener, err := net.Listen("tcp4", h.address)
	if err != nil {
		c.Close()
		derp.Close()
		admission.Close()
		t.Fatal(err)
	}
	front := httptest.NewUnstartedServer(mux)
	front.Listener.Close()
	front.Listener = listener
	front.Start()
	h.address = listener.Addr().String()
	r := &hubRun{c: c, front: front, admission: admission, derp: derp}
	h.run = r
	t.Cleanup(r.close)
	r.shim, err = meshnet.NewShim(front.URL, c.Public())
	if err != nil {
		t.Fatal(err)
	}
	listeners, err := c.Listen(ctx, r.shim.URL(), 49135)
	if err != nil {
		t.Fatal(err)
	}
	r.listener = listeners[0]
	serveEcho(r.listener)
}

type node struct {
	server *tsnet.Server
	secret string
	status *ipnstate.Status
}

func (h *hub) enroll(t *testing.T, name string) *node {
	t.Helper()
	ctx, cancel := context.WithTimeout(t.Context(), 20*time.Second)
	defer cancel()
	secret, identity, err := join.Redeem(ctx, h.run.front.URL, h.run.c.Code())
	if err != nil {
		t.Fatal(err)
	}
	doc, err := join.FetchIdentity(ctx, h.run.front.URL)
	if err != nil {
		t.Fatal(err)
	}
	if err := doc.Verify(identity); err != nil {
		t.Fatal(err)
	}
	s := h.client(t, name, secret)
	status, err := s.Up(ctx)
	if err != nil {
		t.Fatal(err)
	}
	return &node{server: s, secret: secret, status: status}
}

func (h *hub) client(t *testing.T, name, secret string) *tsnet.Server {
	t.Helper()
	shim, err := meshnet.NewShim(h.run.front.URL, h.run.c.Public())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { shim.Close() })
	s := &tsnet.Server{Dir: t.TempDir(), Hostname: name, ControlURL: shim.URL(), AuthKey: secret, Logf: t.Logf, UserLogf: t.Logf}
	t.Cleanup(func() { s.Close() })
	return s
}

func serveEcho(listener net.Listener) {
	go func() {
		for {
			conn, err := listener.Accept()
			if err != nil {
				return
			}
			go func() {
				defer conn.Close()
				conn.SetDeadline(time.Now().Add(5 * time.Second))
				io.Copy(conn, conn)
			}()
		}
	}()
}

func echo(ctx context.Context, s *tsnet.Server, address string) error {
	ctx, cancel := context.WithTimeout(ctx, 500*time.Millisecond)
	defer cancel()
	conn, err := s.Dial(ctx, "tcp", address)
	if err != nil {
		return err
	}
	defer conn.Close()
	deadline, _ := ctx.Deadline()
	conn.SetDeadline(deadline)
	if _, err := io.WriteString(conn, "mesh"); err != nil {
		return err
	}
	var b [4]byte
	if _, err := io.ReadFull(conn, b[:]); err != nil {
		return err
	}
	if string(b[:]) != "mesh" {
		return fmt.Errorf("echo = %q", b)
	}
	return nil
}

func eventually(t *testing.T, ctx context.Context, what string, check func(context.Context) error) {
	t.Helper()
	tick := time.NewTicker(25 * time.Millisecond)
	defer tick.Stop()
	for {
		err := check(ctx)
		if err == nil {
			return
		}
		select {
		case <-ctx.Done():
			t.Fatalf("%s: %v (last check: %v)", what, ctx.Err(), err)
		case <-tick.C:
		}
	}
}

func status(ctx context.Context, s *tsnet.Server) (*ipnstate.Status, error) {
	client, err := s.LocalClient()
	if err != nil {
		return nil, err
	}
	return client.Status(ctx)
}

func TestJoinIsolateRemove(t *testing.T) {
	h := newHub(t)
	a, b := h.enroll(t, "a"), h.enroll(t, "b")
	ctx, cancel := context.WithTimeout(t.Context(), 5*time.Second)
	defer cancel()
	for _, n := range []*node{a, b} {
		eventually(t, ctx, "node can reach hub", func(ctx context.Context) error {
			return echo(ctx, n.server, h.run.listener.Addr().String())
		})
	}
	listener, err := b.server.Listen("tcp6", ":49135")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { listener.Close() })
	serveEcho(listener)
	address := netip.AddrPortFrom(b.status.TailscaleIPs[0], 49135).String()
	before, err := status(t.Context(), a.server)
	if err != nil {
		t.Fatal(err)
	}
	if before.Peer[b.status.Self.PublicKey] != nil {
		t.Fatal("a sees b")
	}
	ctxBefore, cancelBefore := context.WithTimeout(t.Context(), 500*time.Millisecond)
	conn, err := a.server.Dial(ctxBefore, "tcp", address)
	cancelBefore()
	if conn != nil {
		conn.Close()
	}
	if err == nil {
		t.Fatal("a dialed b")
	}

	if err := h.run.c.Remove("a"); err != nil {
		t.Fatal(err)
	}
	a.server.Close()
	fresh := h.client(t, "a", a.secret)
	ctxRefused, cancelRefused := context.WithTimeout(t.Context(), 5*time.Second)
	defer cancelRefused()
	if _, err := fresh.Up(ctxRefused); err == nil || !strings.Contains(err.Error(), join.ErrCode.Error()) {
		t.Fatalf("fresh keys with saved secret: got %v, want wrong device code", err)
	}
	for _, n := range h.run.c.Nodes() {
		if n.Name == "a" {
			t.Fatal("removed a reappeared in coordinator")
		}
	}
	if got := h.redemptions.Load(); got != 2 {
		t.Fatalf("join exchanges = %d, want 2", got)
	}
}

func TestHubRestartKeepsNodes(t *testing.T) {
	h := newHub(t)
	a, b := h.enroll(t, "a"), h.enroll(t, "b")
	ctx, cancel := context.WithTimeout(t.Context(), 5*time.Second)
	defer cancel()
	for _, n := range []*node{a, b} {
		eventually(t, ctx, "node reaches initial hub", func(ctx context.Context) error {
			return echo(ctx, n.server, h.run.listener.Addr().String())
		})
	}
	public, code := h.run.c.Public(), h.run.c.Code()
	hubAddress := h.run.listener.Addr().String()
	h.run.close()
	ctxRestart, cancelRestart := context.WithTimeout(t.Context(), 10*time.Second)
	defer cancelRestart()
	h.start(t, ctxRestart)
	if h.run.c.Public() != public || h.run.c.Code() != code || h.run.listener.Addr().String() != hubAddress {
		t.Fatal("hub identity, code, or overlay address changed across restart")
	}
	eventually(t, ctxRestart, "hub and both existing nodes online after restart", func(ctx context.Context) error {
		nodes := h.run.c.Nodes()
		if len(nodes) != 3 {
			return fmt.Errorf("nodes = %+v, want hub, a, b", nodes)
		}
		for _, n := range nodes {
			if !n.Online {
				return fmt.Errorf("%s is offline", n.Name)
			}
		}
		for _, n := range []*node{a, b} {
			s, err := status(ctx, n.server)
			if err != nil {
				return err
			}
			if s.Self.PublicKey != n.status.Self.PublicKey || s.TailscaleIPs[0] != n.status.TailscaleIPs[0] {
				return errors.New("remote node identity changed")
			}
			if err := echo(ctx, n.server, hubAddress); err != nil {
				return err
			}
		}
		return nil
	})
	if got := h.redemptions.Load(); got != 2 {
		t.Fatalf("join exchanges across hub restart = %d, want initial 2", got)
	}
}
