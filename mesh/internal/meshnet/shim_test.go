package meshnet

import (
	"bufio"
	"context"
	"crypto/rand"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
	"tailscale.com/control/controlhttp"
	"tailscale.com/control/controlhttp/controlhttpcommon"
	"tailscale.com/control/controlhttp/controlhttpserver"
	"tailscale.com/derp"
	"tailscale.com/derp/derphttp"
	"tailscale.com/derp/derpserver"
	"tailscale.com/net/netmon"
	"tailscale.com/tailcfg"
	"tailscale.com/types/key"
	"tailscale.com/types/logger"
)

func newTestShim(t *testing.T, endpoint string, pinned key.MachinePublic) *Shim {
	t.Helper()
	s, err := NewShim(endpoint, pinned)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := s.Close(); err != nil {
			t.Error(err)
		}
	})
	return s
}
func TestShimServesPinnedKey(t *testing.T) {
	pinned := key.NewMachine().Public()
	s := newTestShim(t, "https://example.invalid/mesh/", pinned)
	r, err := http.Get(fmt.Sprintf("%s/key?v=%d", s.URL(), tailcfg.CurrentCapabilityVersion))
	if err != nil {
		t.Fatal(err)
	}
	defer r.Body.Close()
	var got tailcfg.OverTLSPublicKeyResponse
	if err := json.NewDecoder(r.Body).Decode(&got); err != nil {
		t.Fatal(err)
	}
	if got.PublicKey != pinned {
		t.Fatal("key differs from pin")
	}
}
func echoFront(t *testing.T, seen chan *http.Request) *httptest.Server {
	t.Helper()
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		seen <- r.Clone(context.Background())
		c, err := websocket.Accept(w, r, &websocket.AcceptOptions{Subprotocols: []string{"tailscale-control-protocol", "derp"}})
		if err != nil {
			return
		}
		defer c.CloseNow()
		for {
			typ, b, err := c.Read(r.Context())
			if err != nil {
				return
			}
			if err := c.Write(r.Context(), typ, b); err != nil {
				return
			}
		}
	}))
	t.Cleanup(srv.Close)
	return srv
}
func nativeUpgrade(t *testing.T, s *Shim, path string) (net.Conn, *bufio.Reader) {
	t.Helper()
	u, _ := url.Parse(s.URL())
	c, err := net.DialTimeout("tcp", u.Host, 3*time.Second)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { c.Close() })
	c.SetDeadline(time.Now().Add(5 * time.Second))
	protocol := "DERP"
	if path == "/ts2021" {
		protocol = controlhttpcommon.UpgradeHeaderValue
	}
	fmt.Fprintf(c, "GET %s HTTP/1.1\r\nHost: %s\r\nConnection: Upgrade\r\nUpgrade: %s\r\n%s: aGVsbG8=\r\n\r\n", path, u.Host, protocol, controlhttpcommon.HandshakeHeaderName)
	r := bufio.NewReader(c)
	resp, err := http.ReadResponse(r, nil)
	if err != nil {
		t.Fatal(err)
	}
	if resp.StatusCode != 101 {
		t.Fatalf("upgrade: %s", resp.Status)
	}
	return c, r
}
func TestShimBridgesUpgradeToWebSocket(t *testing.T) {
	for _, route := range []string{"/ts2021", "/derp"} {
		t.Run(route, func(t *testing.T) {
			seen := make(chan *http.Request, 1)
			front := echoFront(t, seen)
			s := newTestShim(t, front.URL, key.NewMachine().Public())
			c, r := nativeUpgrade(t, s, route)
			payload := []byte("raw byte stream\x00\xff")
			if _, err := c.Write(payload); err != nil {
				t.Fatal(err)
			}
			b := make([]byte, len(payload))
			if _, err := io.ReadFull(r, b); err != nil {
				t.Fatal(err)
			}
			if string(b) != string(payload) {
				t.Fatal("wrong echo")
			}
			req := <-seen
			if req.URL.Path != route {
				t.Fatal(req.URL)
			}
			if route == "/ts2021" && (req.URL.Query().Get(controlhttpcommon.HandshakeHeaderName) != "aGVsbG8=" || req.Header.Get("X-Iii-Mesh-Derp-Url") == "" || req.Header.Get("X-Iii-Mesh-Derp-Cert") == "") {
				t.Fatal("missing control handshake or relay metadata")
			}
		})
	}
}
func TestShimJoinsPrefixedURL(t *testing.T) {
	seen := make(chan *http.Request, 1)
	front := echoFront(t, seen)
	s := newTestShim(t, front.URL+"/mesh/", key.NewMachine().Public())
	nativeUpgrade(t, s, "/ts2021")
	if req := <-seen; req.URL.Path != "/mesh/ts2021" {
		t.Fatal(req.URL)
	}
}
func TestShimRealNoiseHandshake(t *testing.T) {
	machine := key.NewMachine()
	front := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		c, err := controlhttpserver.AcceptHTTP(r.Context(), w, r, machine, nil)
		if err != nil {
			t.Logf("Noise accept: %v", err)
			return
		}
		defer c.Close()
		io.Copy(c, c)
	}))
	defer front.Close()
	s := newTestShim(t, front.URL+"/mesh/", machine.Public())
	u, _ := url.Parse(s.URL())
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	d := controlhttp.Dialer{Logf: t.Logf, Hostname: u.Hostname(), HTTPPort: u.Port(), HTTPSPort: "none", MachineKey: key.NewMachine(), ControlKey: machine.Public(), ProtocolVersion: uint16(tailcfg.CurrentCapabilityVersion)}
	c, err := d.Dial(ctx)
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	c.SetDeadline(time.Now().Add(5 * time.Second))
	payload := []byte(rand.Text())
	if _, err := c.Write(payload); err != nil {
		t.Fatal(err)
	}
	b := make([]byte, len(payload))
	if _, err := io.ReadFull(c, b); err != nil {
		t.Fatal(err)
	}
	if string(b) != string(payload) {
		t.Fatal("Noise echo differs")
	}
}
func TestShimRealDERPRegionClient(t *testing.T) {
	server := derpserver.New(key.NewNode(), logger.Discard)
	defer server.Close()
	seen := make(chan *http.Request, 1)
	front := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/mesh/ts2021" {
			seen <- r.Clone(context.Background())
			c, err := websocket.Accept(w, r, &websocket.AcceptOptions{Subprotocols: []string{controlhttpcommon.UpgradeHeaderValue}})
			if err == nil {
				defer c.CloseNow()
				c.Read(r.Context())
			}
			return
		}
		if r.URL.Path != "/mesh/derp" {
			http.NotFound(w, r)
			return
		}
		derpserver.AddWebSocketSupport(server, derpserver.Handler(server)).ServeHTTP(w, r)
	}))
	defer front.Close()
	s := newTestShim(t, front.URL+"/mesh/", key.NewMachine().Public())
	nativeUpgrade(t, s, "/ts2021")
	req := <-seen
	u, err := url.Parse(req.Header.Get("X-Iii-Mesh-Derp-Url"))
	if err != nil {
		t.Fatal(err)
	}
	port, err := strconv.Atoi(u.Port())
	if err != nil {
		t.Fatal(err)
	}
	region := &tailcfg.DERPRegion{RegionID: 1, RegionCode: "hub", Nodes: []*tailcfg.DERPNode{{Name: "hub", RegionID: 1, HostName: u.Hostname(), IPv4: u.Hostname(), IPv6: "none", DERPPort: port, STUNPort: -1, CertName: req.Header.Get("X-Iii-Mesh-Derp-Cert")}}}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	akey, bkey := key.NewNode(), key.NewNode()
	a := derphttp.NewRegionClient(akey, logger.Discard, netmon.NewStatic(), func() *tailcfg.DERPRegion { return region })
	defer a.Close()
	b := derphttp.NewRegionClient(bkey, logger.Discard, netmon.NewStatic(), func() *tailcfg.DERPRegion { return region })
	defer b.Close()
	if err := a.Connect(ctx); err != nil {
		t.Fatal(err)
	}
	if err := b.Connect(ctx); err != nil {
		t.Fatal(err)
	}
	if _, err := a.Recv(); err != nil {
		t.Fatal(err)
	}
	if _, err := b.Recv(); err != nil {
		t.Fatal(err)
	}
	payload := []byte(rand.Text())
	if err := a.Send(bkey.Public(), payload); err != nil {
		t.Fatal(err)
	}
	received := make(chan error, 1)
	go func() {
		for {
			msg, err := b.Recv()
			if err != nil {
				received <- err
				return
			}
			if p, ok := msg.(derp.ReceivedPacket); ok {
				if p.Source != akey.Public() || string(p.Data) != string(payload) {
					received <- fmt.Errorf("wrong relayed packet")
				} else {
					received <- nil
				}
				return
			}
		}
	}()
	select {
	case err := <-received:
		if err != nil {
			t.Fatal(err)
		}
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	bad := *region.Nodes[0]
	bad.CertName = "sha256-raw:" + strings.Repeat("0", 64)
	wrong := *region
	wrong.Nodes = []*tailcfg.DERPNode{&bad}
	rejected := derphttp.NewRegionClient(key.NewNode(), logger.Discard, netmon.NewStatic(), func() *tailcfg.DERPRegion { return &wrong })
	defer rejected.Close()
	if err := rejected.Connect(ctx); err == nil {
		t.Fatal("incorrect certificate pin accepted")
	}
}

func TestShimCloseStopsActiveBridges(t *testing.T) {
	seen := make(chan *http.Request, 1)
	front := echoFront(t, seen)
	s := newTestShim(t, front.URL, key.NewMachine().Public())
	c, r := nativeUpgrade(t, s, "/derp")
	<-seen
	done := make(chan error, 1)
	go func() { done <- s.Close() }()
	select {
	case err := <-done:
		if err != nil {
			t.Fatal(err)
		}
	case <-time.After(3 * time.Second):
		t.Fatal("Close left a bridge running")
	}
	c.SetReadDeadline(time.Now().Add(time.Second))
	if _, err := r.ReadByte(); err == nil {
		t.Fatal("closed shim left native connection open")
	}
	if err := s.Close(); err != nil {
		t.Fatal(err)
	}
	if _, err := http.Get(s.URL() + "/key"); err == nil {
		t.Fatal("closed shim still accepts control requests")
	}
}

func TestShimRejectsUnusableEndpoints(t *testing.T) {
	for _, endpoint := range []string{"ftp://example.invalid/", "https://user:pass@example.invalid/", "https://example.invalid/?q=1", "https://example.invalid/#", "/relative"} {
		if s, err := NewShim(endpoint, key.NewMachine().Public()); err == nil {
			s.Close()
			t.Fatalf("accepted %q", endpoint)
		}
	}
	if s, err := NewShim("https://example.invalid", key.MachinePublic{}); err == nil {
		s.Close()
		t.Fatal("accepted missing control pin")
	}
}
