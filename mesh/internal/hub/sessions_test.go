package hub

import (
	"context"
	"net/http"
	"net/http/httptest"
	"net/netip"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/iii-hq/workers/mesh/internal/gateway"
)

func TestAuthConnectionLifetime(t *testing.T) {
	h := authHub()
	keys := make(chan string, 8)
	proxy := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		result, err := h.Auth(r.Context(), AuthRequest{Headers: map[string]string{gateway.TicketHeader: r.Header.Get(gateway.TicketHeader)}})
		if err != nil {
			http.Error(w, err.Error(), 403)
			return
		}
		keys <- result.Context.SessionKey
		if r.URL.Path == "/fail" {
			http.Error(w, "failed upgrade", 400)
			return
		}
		c, err := websocket.Accept(w, r, &websocket.AcceptOptions{InsecureSkipVerify: true})
		if err != nil {
			return
		}
		defer c.CloseNow()
		c.Read(r.Context())
	}))
	defer proxy.Close()
	h.MeshGateway = gateway.New(func(netip.Addr) (string, bool) { return "laptop", true }, "ws"+strings.TrimPrefix(proxy.URL, "http"), h.Tickets)
	defer h.MeshGateway.Close()
	h.SetHooks(Hooks{})
	h.MeshGateway.SetActive(true)
	server := httptest.NewServer(h.MeshGateway)
	defer server.Close()
	url := "ws" + strings.TrimPrefix(server.URL, "http")
	connect := func() (*websocket.Conn, string) {
		t.Helper()
		c, _, err := websocket.Dial(t.Context(), url, nil)
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() { c.CloseNow() })
		return c, <-keys
	}
	a, ak := connect()
	b, bk := connect()
	a.CloseNow()
	waitInvalid(t, h.Sessions, ak)
	if _, ok := h.Sessions.Valid(bk); !ok {
		t.Fatal("disconnect revoked healthy sibling")
	}
	if c, _, err := websocket.Dial(t.Context(), url+"/fail", nil); err == nil {
		c.CloseNow()
		t.Fatal("failed upgrade succeeded")
	}
	waitInvalid(t, h.Sessions, <-keys)
	h.revoke("laptop")
	waitInvalid(t, h.Sessions, bk)
	b.CloseNow()
}
func waitInvalid(t *testing.T, s *Sessions, key string) {
	t.Helper()
	ctx, cancel := context.WithTimeout(t.Context(), time.Second)
	defer cancel()
	for {
		if _, ok := s.Valid(key); !ok {
			return
		}
		select {
		case <-ctx.Done():
			t.Fatal("closed connection retained session")
		case <-time.After(time.Millisecond):
		}
	}
}
func TestSessionCloseBeforeAuthBinding(t *testing.T) {
	h := authHub()
	ownerSeen := make(chan *gateway.Connection, 1)
	proxy := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, owner, ok := h.Tickets.ConsumeOwned(r.Header.Get(gateway.TicketHeader))
		if !ok {
			t.Error("missing ticket")
		}
		ownerSeen <- owner
		http.Error(w, "upgrade failed", 400)
	}))
	defer proxy.Close()
	g := gateway.New(func(netip.Addr) (string, bool) { return "laptop", true }, "ws"+strings.TrimPrefix(proxy.URL, "http"), h.Tickets)
	defer g.Close()
	g.SetActive(true)
	server := httptest.NewServer(g)
	defer server.Close()
	if c, _, err := websocket.Dial(t.Context(), "ws"+strings.TrimPrefix(server.URL, "http"), nil); err == nil {
		c.CloseNow()
		t.Fatal("bad upgrade succeeded")
	}
	owner := <-ownerSeen
	g.CloseConnections()
	if key, live := h.Sessions.openOwned("laptop", owner); live {
		t.Fatal("dead owner revived")
	} else if _, ok := h.Sessions.Valid(key); ok {
		t.Fatal("close-before-binding leaked key")
	}
}
