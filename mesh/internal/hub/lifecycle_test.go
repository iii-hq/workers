package hub

import (
	"context"
	"encoding/binary"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/netip"
	"strings"
	"testing"
	"time"

	"github.com/iii-hq/workers/mesh/internal/join"
	"github.com/iii-hq/workers/mesh/internal/meshnet"
	"tailscale.com/tsnet"
)

func TestRemoveReturnsNewCode(t *testing.T) {
	ctx, cancel := context.WithTimeout(t.Context(), 45*time.Second)
	defer cancel()
	cfg := DefaultConfig()
	cfg.Listen = "127.0.0.1:0"
	cfg.GatewayPort = 50221
	h, err := Start(ctx, Options{DataDir: t.TempDir(), Version: "0.1.0"}, cfg, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer h.Close()
	if status := h.Status(); status.FrontDoor != "ok" || status.Gateway == "ok" || status.Proxy == "ok" || status.Nodes.Total != 0 {
		t.Fatalf("misleading startup status: %+v", status)
	}
	cfg = h.Config()
	cfg.PublicURL = h.FrontURL()
	if err := h.ApplyConfig(ctx, cfg); err != nil {
		t.Fatal(err)
	}
	for _, path := range []string{"/", "/key", "/verify", "/derp/private"} {
		res, err := http.Get(h.FrontURL() + path)
		if err != nil {
			t.Fatal(err)
		}
		res.Body.Close()
		if res.StatusCode != 404 {
			t.Fatalf("public %s: %d", path, res.StatusCode)
		}
	}
	secret, _, err := join.Redeem(ctx, h.FrontURL(), h.Coordinator.Code())
	if err != nil {
		t.Fatal(err)
	}
	shim, err := meshnet.NewShim(h.FrontURL(), h.Coordinator.Public())
	if err != nil {
		t.Fatal(err)
	}
	defer shim.Close()
	node := &tsnet.Server{Dir: t.TempDir(), Hostname: "laptop", ControlURL: shim.URL(), AuthKey: secret, Logf: func(string, ...any) {}, UserLogf: func(string, ...any) {}}
	defer node.Close()
	if _, err := node.Up(ctx); err != nil {
		t.Fatal(err)
	}
	removed := make(chan string, 1)
	changed := make(chan Config, 4)
	h.SetHooks(Hooks{Gateway: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { io.WriteString(w, "gateway") }), RevokeNode: func(name string) { removed <- name }, ConfigChanged: func(ctx context.Context, old, next Config) { changed <- next }, ProxyStatus: func() string { return "ok" }})
	client := &http.Client{Transport: &http.Transport{DialContext: node.Dial, DisableKeepAlives: true}, Timeout: 5 * time.Second}
	defer client.CloseIdleConnections()
	check := func(port int) {
		t.Helper()
		res, err := client.Get(fmt.Sprintf("http://[%s]:%d/", h.Status().HubAddress, port))
		if err != nil {
			t.Fatal(err)
		}
		defer res.Body.Close()
		b, _ := io.ReadAll(res.Body)
		if string(b) != "gateway" {
			t.Fatalf("gateway: %s", b)
		}
	}
	check(50221)
	occupied, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer occupied.Close()
	bad := h.Config()
	bad.Listen = occupied.Addr().String()
	bad.GatewayPort = 50222
	if err := h.ApplyConfig(ctx, bad); err == nil {
		t.Fatal("accepted occupied listen address")
	}
	if h.Config().GatewayPort != 50221 {
		t.Fatal("failed reload changed live config")
	}
	check(50221)
	busy, err := h.Coordinator.ListenTCP(50223)
	if err != nil {
		t.Fatal(err)
	}
	bad = h.Config()
	bad.GatewayPort = 50223
	if err := h.ApplyConfig(ctx, bad); err == nil {
		t.Fatal("occupied overlay port accepted")
	}
	busy.Close()
	if len(changed) != 0 {
		t.Fatal("failed bind notified config subscribers")
	}
	cfg = h.Config()
	cfg.Listen = "127.0.0.1:0"
	cfg.GatewayPort = 50222
	// Force a different address string so the ephemeral listener actually rebinds.
	cfg.Listen = "localhost:0"
	oldFront := h.FrontURL()
	if err := h.ApplyConfig(ctx, cfg); err != nil {
		t.Fatal(err)
	}
	if h.FrontURL() == oldFront {
		t.Fatal("front door not rebound")
	}
	check(50222)
	oldctx, oldcancel := context.WithTimeout(ctx, 400*time.Millisecond)
	defer oldcancel()
	conn, err := node.Dial(oldctx, "tcp", fmt.Sprintf("[%s]:50221", h.Status().HubAddress))
	if conn != nil {
		conn.Close()
	}
	if err == nil {
		t.Fatal("old gateway listener remains")
	}
	list, err := h.ListNodes(ctx, struct{}{})
	if err != nil || len(list) != 1 || list[0].Name != "laptop" || list[0].Version != "" {
		t.Fatalf("nodes: %+v, %v", list, err)
	}
	details, err := h.GetNode(ctx, NodeRequest{Name: "laptop"})
	if err != nil || details.Status != nil || details.StatusError == "" {
		t.Fatalf("invented remote status: %+v, %v", details, err)
	}
	if _, err := h.GetNode(ctx, NodeRequest{Name: "hub"}); err == nil {
		t.Fatal("exposed reserved hub")
	}
	if len(changed) != 1 {
		t.Fatalf("config notifications=%d", len(changed))
	}
	oldCode := h.Coordinator.Code()
	result, err := h.RemoveNode(ctx, NodeRequest{Name: "laptop"})
	if err != nil {
		t.Fatal(err)
	}
	if !result.Removed || result.Code == oldCode {
		t.Fatalf("remove result: %+v", result)
	}
	select {
	case name := <-removed:
		if name != "laptop" {
			t.Fatal(name)
		}
	case <-ctx.Done():
		t.Fatal("missing removal hook")
	}
	node.Close()
	freshShim, err := meshnet.NewShim(h.FrontURL(), h.Coordinator.Public())
	if err != nil {
		t.Fatal(err)
	}
	defer freshShim.Close()
	fresh := &tsnet.Server{Dir: t.TempDir(), Hostname: "laptop", ControlURL: freshShim.URL(), AuthKey: secret, Logf: func(string, ...any) {}, UserLogf: func(string, ...any) {}}
	defer fresh.Close()
	refused, cancelRefused := context.WithTimeout(ctx, 3*time.Second)
	defer cancelRefused()
	if _, err := fresh.Up(refused); err == nil || !strings.Contains(err.Error(), join.ErrCode.Error()) {
		t.Fatalf("saved secret: %v", err)
	}
	if err := h.Close(); err != nil {
		t.Fatal(err)
	}
	if h.Status().FrontDoor == "ok" {
		t.Fatal("closed front door reported healthy")
	}
}

func TestJoinDisabledWithoutPublicURL(t *testing.T) {
	ctx, cancel := context.WithTimeout(t.Context(), 15*time.Second)
	defer cancel()
	cfg := DefaultConfig()
	cfg.Listen = "127.0.0.1:0"
	h, err := Start(ctx, Options{DataDir: t.TempDir()}, cfg, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer h.Close()
	shim, err := meshnet.NewShim(h.FrontURL(), h.Coordinator.Public())
	if err != nil {
		t.Fatal(err)
	}
	defer shim.Close()
	node := &tsnet.Server{Dir: t.TempDir(), Hostname: "laptop", ControlURL: shim.URL(), AuthKey: join.Secret(h.Coordinator.Identity(), h.Coordinator.Code()), Logf: func(string, ...any) {}, UserLogf: func(string, ...any) {}}
	defer node.Close()
	if _, err := node.Up(ctx); err == nil || !strings.Contains(err.Error(), "public_url is not set") {
		t.Fatalf("join with unpublished hub: %v", err)
	}
}

// proxyForwarder stands in for quick-tunnel with proxy_protocol: each connection to
// the returned URL reaches front after a PROXY v2 header naming visitor.
func proxyForwarder(t *testing.T, front string, visitor netip.AddrPort) string {
	t.Helper()
	l, err := net.Listen("tcp4", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { l.Close() })
	src := visitor.Addr().As4()
	header := append(append([]byte{}, proxySignature...), 0x21, 0x11, 0, 12)
	header = append(append(header, src[:]...), 0, 0, 0, 0)
	header = binary.BigEndian.AppendUint16(binary.BigEndian.AppendUint16(header, visitor.Port()), 443)
	go func() {
		for {
			c, err := l.Accept()
			if err != nil {
				return
			}
			go func() {
				defer c.Close()
				up, err := net.Dial("tcp", front)
				if err != nil {
					return
				}
				defer up.Close()
				if _, err := up.Write(header); err != nil {
					return
				}
				go func() { io.Copy(up, c); up.(*net.TCPConn).CloseWrite() }()
				io.Copy(c, up)
			}()
		}
	}()
	return "http://" + l.Addr().String()
}

func postJoin(t *testing.T, base string, header http.Header) int {
	t.Helper()
	req, err := http.NewRequest(http.MethodPost, base+"/join", strings.NewReader("junk"))
	if err != nil {
		t.Fatal(err)
	}
	for k, v := range header {
		req.Header[k] = v
	}
	res, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	res.Body.Close()
	return res.StatusCode
}

// Tunnel visitors behind a PROXY v2 header get their own join buckets, and the
// upgraded control and relay connections work through the wrapped connection.
func TestFrontDoorBehindProxyProtocol(t *testing.T) {
	ctx, cancel := context.WithTimeout(t.Context(), 45*time.Second)
	defer cancel()
	cfg := DefaultConfig()
	cfg.Listen = "127.0.0.1:0"
	cfg.GatewayPort = 50271
	h, err := Start(ctx, Options{DataDir: t.TempDir(), Version: "0.1.0"}, cfg, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer h.Close()
	cfg = h.Config()
	cfg.PublicURL = h.FrontURL()
	if err := h.ApplyConfig(ctx, cfg); err != nil {
		t.Fatal(err)
	}
	h.tunnelProxy.Store(true)
	front := strings.TrimPrefix(h.FrontURL(), "http://")
	a := proxyForwarder(t, front, netip.MustParseAddrPort("203.0.113.7:4000"))
	for i := range 10 {
		if code := postJoin(t, a, nil); code == http.StatusTooManyRequests {
			t.Fatalf("attempt %d refused early", i+1)
		}
	}
	if code := postJoin(t, a, nil); code != http.StatusTooManyRequests {
		t.Fatalf("11th attempt from one visitor: %d", code)
	}
	if code := postJoin(t, proxyForwarder(t, front, netip.MustParseAddrPort("203.0.113.8:4000")), nil); code == http.StatusTooManyRequests {
		t.Fatal("another visitor shares the bucket")
	}
	if code := postJoin(t, h.FrontURL(), nil); code == http.StatusTooManyRequests {
		t.Fatal("a local client without a header shares the visitor's bucket")
	}

	c := proxyForwarder(t, front, netip.MustParseAddrPort("203.0.113.9:4000"))
	secret, _, err := join.Redeem(ctx, c, h.Coordinator.Code())
	if err != nil {
		t.Fatal(err)
	}
	shim, err := meshnet.NewShim(c, h.Coordinator.Public())
	if err != nil {
		t.Fatal(err)
	}
	defer shim.Close()
	node := &tsnet.Server{Dir: t.TempDir(), Hostname: "laptop", ControlURL: shim.URL(), AuthKey: secret, Logf: func(string, ...any) {}, UserLogf: func(string, ...any) {}}
	defer node.Close()
	if _, err := node.Up(ctx); err != nil {
		t.Fatal(err)
	}
	h.SetHooks(Hooks{Gateway: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { io.WriteString(w, "gateway") })})
	client := &http.Client{Transport: &http.Transport{DialContext: node.Dial, DisableKeepAlives: true}, Timeout: 10 * time.Second}
	res, err := client.Get(fmt.Sprintf("http://[%s]:50271/", h.Status().HubAddress))
	if err != nil {
		t.Fatal(err)
	}
	defer res.Body.Close()
	if b, _ := io.ReadAll(res.Body); string(b) != "gateway" {
		t.Fatalf("gateway: %s", b)
	}
}

// B8: while the tunnel runs on OpenTunnel, a forwarding header from loopback is
// the visitor's, so a loopback entry in trusted_proxies must not honor it.
func TestOpenTunnelIgnoresLoopbackForwardingHeaders(t *testing.T) {
	ctx, cancel := context.WithTimeout(t.Context(), 30*time.Second)
	defer cancel()
	cfg := DefaultConfig()
	cfg.Listen = "127.0.0.1:0"
	cfg.GatewayPort = 50272
	cfg.TrustedProxies = []string{"127.0.0.1/32"}
	cfg.PublicURL = "https://hub.example"
	h, err := Start(ctx, Options{DataDir: t.TempDir(), Version: "0.1.0"}, cfg, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer h.Close()
	post := func(cf string) int { return postJoin(t, h.FrontURL(), http.Header{"Cf-Connecting-Ip": {cf}}) }
	for i := 1; i <= 11; i++ {
		if code := post(fmt.Sprintf("198.51.100.%d", i)); code == http.StatusTooManyRequests {
			t.Fatalf("trusted proxy header %d refused", i)
		}
	}
	h.tunnelRaw.Store(true)
	for i := 101; i <= 110; i++ {
		if code := post(fmt.Sprintf("198.51.100.%d", i)); code == http.StatusTooManyRequests {
			t.Fatalf("attempt %d refused early", i-100)
		}
	}
	if code := post("198.51.100.111"); code != http.StatusTooManyRequests {
		t.Fatalf("forged header opened a new bucket: %d", code)
	}
}
