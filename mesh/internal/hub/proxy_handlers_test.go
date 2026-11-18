package hub

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

func TestProxyHandlerNamespaceForwarding(t *testing.T) {
	ctx, cancel := context.WithTimeout(t.Context(), 5*time.Second)
	defer cancel()
	h := authHub()
	key := h.Sessions.Open("laptop")
	done := make(chan map[string]json.RawMessage, 2)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		c, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer c.CloseNow()
		ns := ""
		registered := false
		sent := false
		for {
			_, b, err := c.Read(ctx)
			if err != nil {
				return
			}
			var frame map[string]json.RawMessage
			json.Unmarshal(b, &frame)
			if string(frame["function_id"]) == `"engine::workers::register"` {
				var data map[string]string
				json.Unmarshal(frame["data"], &data)
				ns = data["namespace"]
			}
			if string(frame["type"]) == `"registerfunction"` && string(frame["id"]) == `"mesh::middleware"` {
				registered = true
			}
			if ns != "" && registered && !sent {
				sent = true
				b, _ := json.Marshal(map[string]any{"type": "invokefunction", "invocation_id": "00000000-0000-4000-8000-000000000001", "function_id": "mesh::middleware", "metadata": map[string]any{"source": "proxy"}, "data": map[string]any{"function_id": "shell::exec", "payload": map[string]any{"command": "pwd"}, "action": nil, "namespace": ns, "context": SessionContext{MeshNode: "laptop", SessionKey: key}}})
				if c.Write(ctx, websocket.MessageText, b) != nil {
					return
				}
			}
			if string(frame["function_id"]) == `"shell::exec"` {
				if string(frame["namespace"]) != `"`+ns+`"` || string(frame["metadata"]) != `{"source":"proxy"}` {
					t.Errorf("forwarded namespace/metadata: %s", b)
				}
				b, _ := json.Marshal(map[string]any{"type": "invocationresult", "invocation_id": frame["invocation_id"], "function_id": frame["function_id"], "result": map[string]any{"target": ns}})
				if c.Write(ctx, websocket.MessageText, b) != nil {
					return
				}
			}
			if string(frame["type"]) == `"invocationresult"` {
				done <- frame
			}
		}
	}))
	defer server.Close()
	url := "ws" + strings.TrimPrefix(server.URL, "http")
	h.Client = iii.New(url, iii.WithName("mesh"), iii.WithNamespace("proj"))
	defer h.Client.Close()
	if err := h.Client.Connect(ctx); err != nil {
		t.Fatal(err)
	}
	if err := h.registerProxyFunctions(ctx, url); err != nil {
		t.Fatal(err)
	}
	defer h.Close()
	seen := map[string]bool{}
	for range 2 {
		select {
		case frame := <-done:
			var result map[string]string
			json.Unmarshal(frame["result"], &result)
			seen[result["target"]] = true
		case <-ctx.Done():
			t.Fatal(ctx.Err())
		}
	}
	if !seen["proj"] || !seen["default"] {
		t.Fatalf("target results: %+v", seen)
	}
}
