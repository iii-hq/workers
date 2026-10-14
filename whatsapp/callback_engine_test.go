package main

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/coder/websocket"
	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

// Keep SDK serialization and dispatch real, replacing only the engine transport.
type callbackPeer struct {
	client *iii.Client
	frames chan map[string]json.RawMessage
	conn   *websocket.Conn
	mu     sync.Mutex
	nextID uint64
}

func newCallbackPeer(t *testing.T, respond func(map[string]json.RawMessage) map[string]any) *callbackPeer {
	t.Helper()
	p := &callbackPeer{frames: make(chan map[string]json.RawMessage, 1024)}
	ready := make(chan struct{})
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer conn.CloseNow()
		p.conn = conn
		close(ready)
		for {
			_, body, err := conn.Read(r.Context())
			if err != nil {
				return
			}
			var msg map[string]json.RawMessage
			if json.Unmarshal(body, &msg) != nil {
				return
			}
			p.frames <- msg
			if respond != nil {
				if reply := respond(msg); reply != nil {
					if p.write(r.Context(), reply) != nil {
						return
					}
				}
			}
		}
	}))
	t.Cleanup(server.Close)
	p.client = iii.New("ws"+strings.TrimPrefix(server.URL, "http"), iii.WithNamespace("default"))
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	if err := p.client.Connect(ctx); err != nil {
		t.Fatal(err)
	}
	select {
	case <-ready:
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	t.Cleanup(func() { _ = p.client.Close() })
	return p
}

func (p *callbackPeer) write(ctx context.Context, value any) error {
	body, err := json.Marshal(value)
	if err != nil {
		return err
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	return p.conn.Write(ctx, websocket.MessageText, body)
}

func (p *callbackPeer) invoke(ctx context.Context, id string, raw json.RawMessage) (json.RawMessage, *iii.InvocationError, error) {
	p.nextID++
	invocationID := fmt.Sprintf("00000000-0000-4000-8000-%012d", p.nextID)
	if err := p.write(ctx, map[string]any{"type": "invokefunction", "function_id": id, "invocation_id": invocationID, "data": raw}); err != nil {
		return nil, nil, err
	}
	for {
		select {
		case frame := <-p.frames:
			if string(frame["invocation_id"]) != fmt.Sprintf("%q", invocationID) || string(frame["type"]) != `"invocationresult"` {
				continue
			}
			var remote *iii.InvocationError
			if len(frame["error"]) > 0 && string(frame["error"]) != "null" {
				remote = &iii.InvocationError{}
				if err := json.Unmarshal(frame["error"], remote); err != nil {
					return nil, nil, err
				}
			}
			return frame["result"], remote, nil
		case <-ctx.Done():
			return nil, nil, ctx.Err()
		}
	}
}

func callbackInvocationReply(msg map[string]json.RawMessage) map[string]any {
	return map[string]any{"type": "invocationresult", "function_id": msg["function_id"], "invocation_id": msg["invocation_id"]}
}
