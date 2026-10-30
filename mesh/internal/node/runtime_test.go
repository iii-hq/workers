package node

import (
	"context"
	"encoding/json"
	"errors"
	"net"
	"net/http"
	"net/http/httptest"
	"strconv"
	"sync"
	"testing"
	"time"

	"github.com/coder/websocket"
)

func TestRunWaitsForGatewayAndReconnectsWorkers(t *testing.T) {
	n, k, m, w := testNode(t)
	m.port = 0
	w.runtimes = make(chan Runtime, 8)
	ctx, cancel := context.WithTimeout(t.Context(), 5*time.Second)
	defer cancel()
	registrations := make(chan int, 8)
	var mu sync.Mutex
	connections := 0
	server := func() *httptest.Server {
		return httptest.NewServer(http.HandlerFunc(func(rw http.ResponseWriter, r *http.Request) {
			c, err := websocket.Accept(rw, r, nil)
			if err != nil {
				return
			}
			defer c.CloseNow()
			mu.Lock()
			connections++
			mu.Unlock()
			seen := map[string]bool{}
			namespace := ""
			notified := false
			for {
				_, raw, err := c.Read(ctx)
				if err != nil {
					return
				}
				var msg map[string]json.RawMessage
				json.Unmarshal(raw, &msg)
				if string(msg["type"]) == `"registerfunction"` {
					var id string
					json.Unmarshal(msg["id"], &id)
					seen[id] = true
				}
				if string(msg["function_id"]) == `"engine::workers::register"` {
					var metadata map[string]string
					json.Unmarshal(msg["data"], &metadata)
					namespace = metadata["namespace"]
				}
				if len(seen) == 4 && namespace != "" && !notified {
					if namespace != "default" || !seen["mesh-node::status"] || !seen["mesh-node::update"] || !seen["mesh-node::workers::apply"] || !seen["mesh-node::drives::apply"] {
						t.Errorf("registration namespace=%q handlers=%v", namespace, seen)
					}
					notified = true
					_, p, _ := net.SplitHostPort(r.Host)
					port, _ := strconv.Atoi(p)
					registrations <- port
				}
			}
		}))
	}
	a, b := server(), server()
	defer a.Close()
	defer b.Close()
	oldTransport := http.DefaultTransport
	transport := oldTransport.(*http.Transport).Clone()
	transport.Proxy = nil
	transport.DialContext = func(ctx context.Context, network, address string) (net.Conn, error) {
		_, port, err := net.SplitHostPort(address)
		if err != nil {
			return nil, err
		}
		return (&net.Dialer{}).DialContext(ctx, network, net.JoinHostPort("127.0.0.1", port))
	}
	http.DefaultTransport = transport
	defer func() { http.DefaultTransport = oldTransport; transport.CloseIdleConnections() }()
	t.Setenv("III_NAMESPACE", "wrong-project")
	done := make(chan error, 1)
	go func() { done <- n.Run(ctx) }()
	select {
	case <-registrations:
		t.Fatal("connected without authenticated gateway")
	case <-time.After(40 * time.Millisecond):
	}
	portOf := func(s *httptest.Server) uint16 {
		_, p, _ := net.SplitHostPort(s.Listener.Addr().String())
		v, _ := strconv.Atoi(p)
		return uint16(v)
	}
	m.setPort(portOf(a))
	select {
	case got := <-registrations:
		if got != int(portOf(a)) {
			t.Fatal(got)
		}
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	s, _ := Sign(k, "mesh-node::workers::apply", WorkersRequest{Namespace: "real-project", Workers: []Worker{}}, time.Now())
	if _, err := n.handle(ctx, "mesh-node::workers::apply", s); err != nil {
		t.Fatal(err)
	}
	<-w.runtimes
	m.setPort(portOf(b))
	select {
	case got := <-registrations:
		if got != int(portOf(b)) {
			t.Fatal(got)
		}
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	select {
	case got := <-w.runtimes:
		if got.URL != GatewayURL(portOf(b)) || got.Namespace != "real-project" || got.Node != "laptop-2" {
			t.Fatal(got)
		}
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	cancel()
	if err := <-done; err != nil {
		t.Fatal(err)
	}
	mu.Lock()
	defer mu.Unlock()
	if connections != 2 {
		t.Fatalf("connections %d", connections)
	}
}
func TestRunReturnsRestartAndRemoval(t *testing.T) {
	for _, reason := range []string{"restart", "removed"} {
		t.Run(reason, func(t *testing.T) {
			n, _, m, _ := testNode(t)
			m.port = 0
			want := ErrRestartRequired
			if reason == "restart" {
				close(n.restart)
			} else {
				close(m.removed)
				want = ErrRemoved
			}
			if err := n.Run(t.Context()); !errors.Is(err, want) {
				t.Fatalf("got %v want %v", err, want)
			}
		})
	}
}
