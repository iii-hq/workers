package hub

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
)

func TestRunRegistersInterfaceAndCentralConfiguration(t *testing.T) {
	for _, mode := range []string{"collect", "local", "central"} {
		t.Run(mode, func(t *testing.T) {
			t.Setenv("III_NAMESPACE", "project")
			t.Setenv("III_CONFIG_NAME", " project_mesh ")
			ctx, cancel := context.WithTimeout(t.Context(), 20*time.Second)
			defer cancel()
			frames := make(chan map[string]json.RawMessage, 64)
			cfg := DefaultConfig()
			cfg.Listen = "127.0.0.1:0"
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				conn, err := websocket.Accept(w, r, nil)
				if err != nil {
					return
				}
				defer conn.CloseNow()
				for {
					_, b, err := conn.Read(ctx)
					if err != nil {
						return
					}
					var frame map[string]json.RawMessage
					if json.Unmarshal(b, &frame) != nil {
						return
					}
					frames <- frame
					if string(frame["type"]) != `"invokefunction"` || len(frame["invocation_id"]) == 0 {
						continue
					}
					var value any = map[string]any{}
					switch string(frame["function_id"]) {
					case `"configuration::get"`:
						value = map[string]any{"value": cfg}
					case `"engine::functions::list"`:
						value = map[string]any{"functions": []any{map[string]string{"function_id": "analytics::report"}}}
					}
					b, _ = json.Marshal(map[string]any{"type": "invocationresult", "invocation_id": frame["invocation_id"], "function_id": frame["function_id"], "result": value})
					if conn.Write(ctx, websocket.MessageText, b) != nil {
						return
					}
				}
			}))
			defer server.Close()
			dir := filepath.Join(t.TempDir(), "state")
			done := make(chan error, 1)
			go func() {
				done <- Run(ctx, Options{URL: "ws" + strings.TrimPrefix(server.URL, "http"), Version: "0.1.0", DataDir: dir, CollectInterface: mode == "collect", LocalConfig: mode == "local"})
			}()
			names := map[string]bool{}
			gets := 0
			bound := false
			for len(names) < 8 || mode == "central" && (!bound || gets < 2) {
				select {
				case f := <-frames:
					switch string(f["type"]) {
					case `"registerfunction"`:
						var id string
						json.Unmarshal(f["id"], &id)
						names[id] = true
					case `"registertrigger"`:
						bound = true
						if string(f["namespace"]) != `"project"` {
							t.Fatalf("trigger namespace: %s", f["namespace"])
						}
						var c map[string]any
						json.Unmarshal(f["config"], &c)
						if c["configuration_id"] != "project_mesh" {
							t.Fatalf("trigger filter: %+v", c)
						}
					case `"invokefunction"`:
						id := string(f["function_id"])
						if mode != "central" && id != `"engine::workers::register"` {
							t.Fatalf("interface mode invoked backend %s", id)
						}
						if strings.Contains(id, "configuration::") {
							if string(f["namespace"]) != `"default"` {
								t.Fatal("configuration outside default")
							}
							var p map[string]any
							json.Unmarshal(f["data"], &p)
							if p["id"] != "project_mesh" {
								t.Fatalf("configuration ID: %+v", p)
							}
						}
						if id == `"configuration::get"` {
							gets++
						}
					}
				case err := <-done:
					t.Fatalf("run ended before registration: %v", err)
				case <-ctx.Done():
					t.Fatal(ctx.Err())
				}
			}
			for _, id := range []string{"mesh::status", "mesh::nodes::list", "mesh::nodes::get", "mesh::nodes::remove", "mesh::join::code", "mesh::join::cycle", "mesh::configuration-id", "mesh::on-config-change"} {
				if !names[id] {
					t.Fatalf("missing %s", id)
				}
			}
			if mode != "central" {
				if _, err := os.Stat(dir); !os.IsNotExist(err) {
					t.Fatalf("interface mode touched state: %v", err)
				}
			}
			cancel()
			if err := <-done; err != nil {
				t.Fatal(err)
			}
		})
	}
}
