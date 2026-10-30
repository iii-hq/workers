package hub

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/coder/websocket"
)

func TestRunRegistersInterfaceAndCentralConfiguration(t *testing.T) {
	for _, mode := range []string{"collect", "local", "central", "central-default"} {
		t.Run(mode, func(t *testing.T) {
			ns := "project"
			if mode == "central-default" {
				ns = "default"
			}
			central := strings.HasPrefix(mode, "central")
			t.Setenv("III_NAMESPACE", ns)
			t.Setenv("III_CONFIG_NAME", " project_mesh ")
			ctx, cancel := context.WithTimeout(t.Context(), 20*time.Second)
			defer cancel()
			frames := make(chan map[string]json.RawMessage, 128)
			var proxyMu sync.Mutex
			var proxyValue any
			cfg := DefaultConfig()
			cfg.Listen = "127.0.0.1:0"
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				conn, err := websocket.Accept(w, r, nil)
				if err != nil {
					return
				}
				defer conn.CloseNow()
				connectionNamespace := ""
				workerName := ""
				var pending []map[string]json.RawMessage
				for {
					_, b, err := conn.Read(ctx)
					if err != nil {
						return
					}
					var frame map[string]json.RawMessage
					if json.Unmarshal(b, &frame) != nil {
						return
					}
					if string(frame["function_id"]) == `"engine::workers::register"` {
						var data map[string]any
						json.Unmarshal(frame["data"], &data)
						connectionNamespace, _ = data["namespace"].(string)
						workerName, _ = data["name"].(string)
						for _, early := range pending {
							early["connection_namespace"], _ = json.Marshal(connectionNamespace)
							early["worker_name"], _ = json.Marshal(workerName)
							frames <- early
						}
						pending = nil
					}
					if connectionNamespace == "" && string(frame["type"]) == `"registerfunction"` {
						pending = append(pending, frame)
						continue
					}
					frame["connection_namespace"], _ = json.Marshal(connectionNamespace)
					frame["worker_name"], _ = json.Marshal(workerName)
					frames <- frame
					if string(frame["type"]) != `"invokefunction"` || len(frame["invocation_id"]) == 0 {
						continue
					}
					var value any = map[string]any{}
					var payload map[string]any
					json.Unmarshal(frame["data"], &payload)
					switch string(frame["function_id"]) {
					case `"configuration::get"`:
						value = map[string]any{"value": cfg}
						if payload["id"] == "rbac-proxy" {
							proxyMu.Lock()
							value = map[string]any{"value": proxyValue}
							proxyMu.Unlock()
						}
					case `"configuration::ensure"`:
						if payload["id"] == "rbac-proxy" {
							proxyMu.Lock()
							if proxyValue == nil {
								proxyValue = payload["initial_value"]
							}
							proxyMu.Unlock()
						}
					case `"rbac-proxy::on-config-change"`:
						proxyMu.Lock()
						applied, _ := proxyValue.(map[string]any)
						value = map[string]any{"ok": true, "applied_config": applied, "bound_host": applied["host"], "bound_port": applied["port"]}
						proxyMu.Unlock()
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
			defaultNames := map[string]bool{}
			proxyGets := 0
			proxyApplies := 0
			proxyBound := false
			gets := 0
			bound := false
			for len(names) < 11 || len(defaultNames) < 2 || central && (!bound || !proxyBound || gets < 2 || proxyGets < 2 || proxyApplies < 1) {
				select {
				case f := <-frames:
					switch string(f["type"]) {
					case `"registerfunction"`:
						var id string
						json.Unmarshal(f["id"], &id)
						var connectionNS string
						json.Unmarshal(f["connection_namespace"], &connectionNS)
						t.Logf("registration %s namespace=%q", id, connectionNS)
						if connectionNS == ns {
							names[id] = true
						}
						if connectionNS == "default" {
							defaultNames[id] = true
						}
						if strings.HasPrefix(id, "mesh::") && (id == "mesh::auth" || id == "mesh::middleware" || id == "mesh::on-trigger-registration") {
							var md map[string]any
							json.Unmarshal(f["metadata"], &md)
							if md["internal"] != true {
								t.Fatal("proxy function is not internal")
							}
						}
					case `"registertrigger"`:
						bound = true
						var gotNS string
						json.Unmarshal(f["namespace"], &gotNS)
						if gotNS != ns {
							t.Fatalf("trigger namespace: %s", f["namespace"])
						}
						var c map[string]any
						json.Unmarshal(f["config"], &c)
						if c["configuration_id"] == "rbac-proxy" {
							proxyBound = true
						} else if c["configuration_id"] != "project_mesh" {
							t.Fatalf("trigger filter: %+v", c)
						}
					case `"invokefunction"`:
						id := string(f["function_id"])
						if !central && id != `"engine::workers::register"` {
							t.Fatalf("interface mode invoked backend %s", id)
						}
						if id == `"rbac-proxy::on-config-change"` {
							proxyApplies++
							var callNS string
							json.Unmarshal(f["namespace"], &callNS)
							if callNS != ns {
								t.Fatalf("proxy apply outside primary namespace: %s", callNS)
							}
						}
						if strings.Contains(id, "configuration::") {
							if string(f["namespace"]) != `"default"` {
								t.Fatal("configuration outside default")
							}
							var p map[string]any
							json.Unmarshal(f["data"], &p)
							if p["id"] != "project_mesh" && p["id"] != "rbac-proxy" {
								t.Fatalf("configuration ID: %+v", p)
							}
						}
						if id == `"configuration::get"` {
							var p map[string]any
							json.Unmarshal(f["data"], &p)
							if p["id"] == "rbac-proxy" {
								proxyGets++
							} else {
								gets++
							}
						}
					}
				case err := <-done:
					t.Fatalf("run ended before registration: %v", err)
				case <-ctx.Done():
					t.Fatalf("%v primary=%v default=%v", ctx.Err(), names, defaultNames)
				}
			}
			for _, id := range []string{"mesh::status", "mesh::nodes::list", "mesh::nodes::get", "mesh::nodes::remove", "mesh::join::code", "mesh::join::cycle", "mesh::configuration-id", "mesh::on-config-change", "mesh::auth", "mesh::middleware", "mesh::on-trigger-registration"} {
				if !names[id] {
					t.Fatalf("missing %s", id)
				}
			}
			if !defaultNames["mesh::middleware"] || !defaultNames["mesh::on-trigger-registration"] {
				t.Fatal("missing shared default handlers")
			}
			if ns != "default" && defaultNames["mesh::auth"] {
				t.Fatal("auth registered in wrong namespace")
			}
			if !central {
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
