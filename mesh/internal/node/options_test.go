package node

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
)

func TestURLOverrideKeepsGatewayGateAndDropsToken(t *testing.T) {
	t.Setenv("III_TOKEN", "local-engine-token")
	ctx, cancel := context.WithTimeout(t.Context(), 5*time.Second)
	defer cancel()
	auth := make(chan []string, 8)
	server := httptest.NewServer(http.HandlerFunc(func(rw http.ResponseWriter, r *http.Request) {
		auth <- r.Header.Values("Authorization")
		c, err := websocket.Accept(rw, r, nil)
		if err != nil {
			return
		}
		defer c.CloseNow()
		for {
			if _, _, err := c.Read(ctx); err != nil {
				return
			}
		}
	}))
	defer server.Close()
	override := "ws" + strings.TrimPrefix(server.URL, "http")
	n, k, m, w := testNodeWith(t, Options{URL: override})
	m.port = 0
	apply, _ := Sign(k, "mesh-node::workers::apply", WorkersRequest{Namespace: "project"}, time.Now())
	if _, err := n.handle(ctx, "mesh-node::workers::apply", apply); err == nil {
		t.Fatal("override bypassed the authenticated gateway gate")
	}
	done := make(chan error, 1)
	go func() { done <- n.Run(ctx) }()
	select {
	case <-auth:
		t.Fatal("connected without authenticated gateway")
	case <-time.After(40 * time.Millisecond):
	}
	m.setPort(50222)
	select {
	case got := <-auth:
		if len(got) != 0 {
			t.Fatalf("Authorization leaked to hub: %q", got)
		}
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	apply, _ = Sign(k, "mesh-node::workers::apply", WorkersRequest{Namespace: "project"}, time.Now())
	if _, err := n.handle(ctx, "mesh-node::workers::apply", apply); err != nil {
		t.Fatal(err)
	}
	w.mu.Lock()
	got := w.runtime
	w.mu.Unlock()
	if got != (Runtime{Node: "laptop-2", Namespace: "project", URL: override}) {
		t.Fatalf("runtime %+v", got)
	}
	cancel()
	if err := <-done; err != nil {
		t.Fatal(err)
	}
}

// WantedVersion keeps the hub's version while it differs from the node's; a
// matching version, or a hub that sends none, clears it.
func TestStatusRequestRecordsHubVersion(t *testing.T) {
	n, k, _, _ := testNode(t)
	for _, c := range []struct {
		request StatusRequest
		want    string
	}{{StatusRequest{HubVersion: "0.2.0"}, "0.2.0"}, {StatusRequest{HubVersion: "0.1.0"}, ""}, {StatusRequest{}, ""}} {
		s, _ := Sign(k, "mesh-node::status", c.request, time.Now())
		if _, err := n.handle(t.Context(), "mesh-node::status", s); err != nil {
			t.Fatal(err)
		}
		if got := n.WantedVersion(); got != c.want {
			t.Fatalf("%+v: wanted version %q, want %q", c.request, got, c.want)
		}
	}
}
