package hub

import (
	"context"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/iii-hq/workers/mesh/internal/gateway"
	"github.com/iii-hq/workers/mesh/internal/join"
	"github.com/iii-hq/workers/mesh/internal/meshnet"
	"tailscale.com/tsnet"
)

func TestLiveGatewayOwnershipLifecycle(t *testing.T) {
	// A hang guard only: under -race this test runs 33 to 36 s, and the dial right after the port move waits one SYN retransmit for the hub's new filter.
	ctx, cancel := context.WithTimeout(t.Context(), 60*time.Second)
	defer cancel()
	seen := make(chan string, 8)
	proxy := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		seen <- r.Header.Get(gateway.TicketHeader)
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
	cfg := DefaultConfig()
	cfg.Listen = "127.0.0.1:0"
	cfg.GatewayPort = 50331
	cfg.ProxyURL = "ws" + strings.TrimPrefix(proxy.URL, "http")
	h, err := Start(ctx, Options{DataDir: t.TempDir()}, cfg, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer h.Close()
	cfg = h.Config()
	cfg.PublicURL = h.FrontURL()
	if err := h.ApplyConfig(ctx, cfg); err != nil {
		t.Fatal(err)
	}
	shim, err := meshnet.NewShim(h.FrontURL(), h.Coordinator.Public())
	if err != nil {
		t.Fatal(err)
	}
	defer shim.Close()
	waitPeerPath := func(peer *tsnet.Server) {
		t.Helper()
		address := fmt.Sprintf("[%s]:%d", h.Status().HubAddress, h.Config().GatewayPort)
		if err := gatewayPeerPath(ctx, peer, address); err != nil {
			t.Fatal(err)
		}
	}
	node := &tsnet.Server{Dir: t.TempDir(), Hostname: "laptop", ControlURL: shim.URL(), AuthKey: join.Secret(h.Coordinator.Identity(), h.Coordinator.Code()), Logf: t.Logf, UserLogf: t.Logf}
	defer node.Close()
	if _, err := node.Up(ctx); err != nil {
		t.Fatal(err)
	}
	waitPeerPath(node)
	client := &http.Client{Transport: &http.Transport{DialContext: node.Dial}, Timeout: 3 * time.Second}
	defer client.CloseIdleConnections()
	endpoint := func() string { return fmt.Sprintf("ws://[%s]:%d/", h.Status().HubAddress, h.Config().GatewayPort) }
	if h.Status().Gateway == "ok" {
		t.Fatal("unowned gateway healthy")
	}
	dialStart := time.Now()
	if c, res, err := websocket.Dial(ctx, endpoint(), &websocket.DialOptions{HTTPClient: client}); err == nil || res == nil || res.StatusCode != 503 {
		if c != nil {
			c.CloseNow()
		}
		if lc, statusErr := node.LocalClient(); statusErr == nil {
			if status, statusErr := lc.Status(ctx); statusErr == nil {
				for _, peer := range status.Peer {
					t.Logf("after initial dial %s: peer=%s online=%t active=%t rx=%d tx=%d handshake=%v", time.Since(dialStart), peer.DNSName, peer.Online, peer.Active, peer.RxBytes, peer.TxBytes, peer.LastHandshake)
				}
			}
		}
		t.Fatalf("unowned connection: %v %v", res, err)
	}
	var revokes atomic.Int32
	changes, closes := 0, 0
	revokeEvents := make(chan string, 4)
	h.SetHooks(Hooks{RevokeNode: func(name string) { revokes.Add(1); revokeEvents <- name }, ConfigChanged: func(context.Context, Config, Config) { changes++ }, Close: func() error { closes++; return nil }, ProxyStatus: func() string { return "fixture-owned" }})
	h.SetHooks(Hooks{RevokeNode: func(name string) { revokes.Add(1); revokeEvents <- name }, ConfigChanged: func(context.Context, Config, Config) { changes++ }, Close: func() error { closes++; return nil }, ProxyStatus: func() string { return "fixture-owned" }})
	h.MeshGateway.SetActive(true)
	if s := h.Status(); s.Gateway != "ok" || s.Proxy != "fixture-owned" {
		t.Fatalf("active status: %+v", s)
	}
	connect := func(busClient *http.Client, nodeName string) (*websocket.Conn, <-chan struct{}) {
		t.Helper()
		c, _, err := websocket.Dial(ctx, endpoint(), &websocket.DialOptions{HTTPClient: busClient, HTTPHeader: http.Header{gateway.TicketHeader: {"forged"}}})
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() { c.CloseNow() })
		ticket := <-seen
		name, owner, ok := h.Tickets.ConsumeOwned(ticket)
		if !ok || name != nodeName {
			t.Fatalf("live identity %q %v", name, ok)
		}
		key, live := h.Sessions.openOwned(name, owner)
		if !live {
			t.Fatal("live auth owner refused")
		}
		done := make(chan struct{})
		owner.OnClose(func() {
			if _, ok := h.Sessions.Valid(key); ok {
				t.Error("closed gateway retained auth session")
			}
			close(done)
		})
		if err := c.Write(ctx, websocket.MessageText, []byte("mesh")); err != nil {
			t.Fatal(err)
		}
		_, b, err := c.Read(ctx)
		if err != nil || string(b) != "mesh" {
			t.Fatalf("copy %q %v", b, err)
		}
		return c, done
	}
	waitClosed := func(done <-chan struct{}) {
		t.Helper()
		select {
		case <-done:
		case <-time.After(time.Second):
			t.Fatal("lifecycle did not close owner")
		}
	}
	a, done := connect(client, "laptop")
	a.CloseNow()
	waitClosed(done)
	stable, live := connect(client, "laptop")
	busy, err := h.Coordinator.ListenTCP(50333)
	if err != nil {
		t.Fatal(err)
	}
	bad := h.Config()
	bad.GatewayPort = 50333
	bad.ProxyURL += "/uncommitted"
	if err := h.ApplyConfig(ctx, bad); err == nil {
		t.Fatal("occupied gateway port accepted")
	}
	busy.Close()
	select {
	case <-live:
		t.Fatal("failed reload closed live owner")
	default:
	}
	if h.Status().Gateway != "ok" || h.Config().GatewayPort != 50331 {
		t.Fatal("failed reload changed gateway")
	}
	if err := stable.Write(ctx, websocket.MessageText, []byte("stable")); err != nil {
		t.Fatal(err)
	}
	if _, b, err := stable.Read(ctx); err != nil || string(b) != "stable" {
		t.Fatalf("rollback lost traffic %q %v", b, err)
	}
	done = live
	cfg = h.Config()
	cfg.GatewayPort = 50332
	if err := h.ApplyConfig(ctx, cfg); err != nil {
		t.Fatal(err)
	}
	waitClosed(done)
	_, done = connect(client, "laptop")
	h.SetGrantDefaults(GrantDefaults{Allow: []string{"shell::exec"}})
	waitClosed(done)
	_, done = connect(client, "laptop")
	cfg = h.Config()
	cfg.ProxyURL += "/next"
	if err := h.ApplyConfig(ctx, cfg); err != nil {
		t.Fatal(err)
	}
	waitClosed(done)
	if h.Status().Gateway == "ok" {
		t.Fatal("changed upstream retained ownership")
	}
	h.MeshGateway.SetActive(true)
	_, done = connect(client, "laptop")
	desktop := &tsnet.Server{Dir: t.TempDir(), Hostname: "desktop", ControlURL: shim.URL(), AuthKey: join.Secret(h.Coordinator.Identity(), h.Coordinator.Code()), Logf: func(string, ...any) {}, UserLogf: func(string, ...any) {}}
	defer desktop.Close()
	if _, err := desktop.Up(ctx); err != nil {
		t.Fatal(err)
	}
	waitPeerPath(desktop)
	desktopClient := &http.Client{Transport: &http.Transport{DialContext: desktop.Dial}, Timeout: 3 * time.Second}
	defer desktopClient.CloseIdleConnections()
	sibling, siblingDone := connect(desktopClient, "desktop")
	<-revokeEvents
	if _, err := h.RemoveNode(ctx, NodeRequest{Name: "laptop"}); err != nil {
		t.Fatal(err)
	}
	waitClosed(done)
	select {
	case name := <-revokeEvents:
		if name != "laptop" {
			t.Fatal(name)
		}
	case <-ctx.Done():
		t.Fatal("missing removal callback")
	}
	if err := sibling.Write(ctx, websocket.MessageText, []byte("healthy")); err != nil {
		t.Fatal(err)
	}
	if _, b, err := sibling.Read(ctx); err != nil || string(b) != "healthy" {
		t.Fatalf("unrelated node was revoked: %q %v", b, err)
	}
	if err := h.Close(); err != nil {
		t.Fatal(err)
	}
	waitClosed(siblingDone)
	if closes != 1 || revokes.Load() != 2 || changes != 2 {
		t.Fatalf("combined hooks: closes=%d revokes=%d changes=%d", closes, revokes.Load(), changes)
	}
}
