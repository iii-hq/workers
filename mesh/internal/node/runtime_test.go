package node

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/coder/websocket"
)

func TestRunWaitsForGatewayAndReconnectsWorkers(t *testing.T) {
	t.Setenv("OTEL_ENABLED", "true")
	ctx, cancel := context.WithTimeout(t.Context(), 5*time.Second)
	defer cancel()
	registrations := make(chan struct{}, 8)
	var mu sync.Mutex
	connections := 0
	server := httptest.NewServer(http.HandlerFunc(func(rw http.ResponseWriter, r *http.Request) {
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
			if len(seen) == 2 && namespace != "" && !notified {
				if namespace != "default" || !seen["mesh-node::status"] || !seen["mesh-node::workers::apply"] {
					t.Errorf("registration namespace=%q handlers=%v", namespace, seen)
				}
				notified = true
				registrations <- struct{}{}
			}
		}
	}))
	defer server.Close()
	override := "ws" + strings.TrimPrefix(server.URL, "http")
	n, k, m, w := testNodeWith(t, Options{URL: override})
	m.port = 0
	w.runtimes = make(chan Runtime, 8)
	t.Setenv("III_NAMESPACE", "wrong-project")
	done := make(chan error, 1)
	go func() { done <- n.Run(ctx) }()
	select {
	case <-registrations:
		t.Fatal("connected without authenticated gateway")
	case <-time.After(40 * time.Millisecond):
	}
	runtime := func(want Runtime) {
		t.Helper()
		select {
		case got := <-w.runtimes:
			if got != want {
				t.Fatalf("runtime %+v, want %+v", got, want)
			}
		case <-ctx.Done():
			t.Fatal(ctx.Err())
		}
	}
	registered := func() {
		t.Helper()
		select {
		case <-registrations:
		case <-ctx.Done():
			t.Fatal(ctx.Err())
		}
	}
	m.setPort(50222)
	registered()
	s, _ := Sign(k, "mesh-node::workers::apply", WorkersRequest{Namespace: "real-project"}, time.Now())
	if _, err := n.handle(ctx, "mesh-node::workers::apply", s); err != nil {
		t.Fatal(err)
	}
	runtime(Runtime{Node: "laptop-2", Namespace: "real-project", URL: override})
	m.setPort(0)
	runtime(Runtime{Node: "laptop-2", Namespace: "real-project"})
	m.setPort(50223)
	registered()
	runtime(Runtime{Node: "laptop-2", Namespace: "real-project", URL: override})
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
func TestRunReturnsRemoval(t *testing.T) {
	n, _, m, _ := testNode(t)
	m.port = 0
	close(m.removed)
	if err := n.Run(t.Context()); !errors.Is(err, ErrRemoved) {
		t.Fatalf("got %v want %v", err, ErrRemoved)
	}
}
