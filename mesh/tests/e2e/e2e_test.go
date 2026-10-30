//go:build e2e

package e2e

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestE2EBootstrapIDEDefaultNamespace(t *testing.T) { bootstrapIDE(t, "default") }
func TestE2EBootstrapIDEProjectNamespace(t *testing.T) { bootstrapIDE(t, "project") }

func bootstrapIDE(t *testing.T, ns string) {
	f := newFixture(t, ns)
	n := f.enroll("laptop")
	root := f.startIDE(n)
	eventually(t, 30*time.Second, "remote shell invocation", func() error {
		b, err := call(f.ctx, f.admin, ns, "laptop::shell::exec", map[string]any{"command": "printf mesh-bootstrap", "cwd": root})
		if err != nil {
			return err
		}
		if !strings.Contains(string(b), "mesh-bootstrap") {
			return fmt.Errorf("shell: %s", b)
		}
		return nil
	})

	f.startHarness()
	f.seedTurn("bootstrap", []string{"laptop::shell::exec"}, t.TempDir())
	result := f.harnessCall("bootstrap", "laptop::shell::exec", map[string]any{"command": "printf harness-bootstrap", "cwd": root})
	if !strings.Contains(string(result), "harness-bootstrap") || !strings.Contains(string(result), `"is_error":false`) {
		t.Fatalf("harness remote shell: %s", result)
	}
	frames := n.snapshot()

	observed := map[string]bool{}
	for _, frame := range frames {
		if frame.Direction == "node-to-hub" && frame.Data["type"] == "invokefunction" {
			id, _ := frame.Data["function_id"].(string)
			p, _ := frame.Data["data"].(map[string]any)
			if (id == "configuration::get" || id == "configuration::ensure") && p["id"] == "laptop__ide" {
				if frame.Data["namespace"] != "default" {
					t.Fatalf("configuration escaped default: %s", mustJSON(frame.Data))
				}
				observed[id] = true
			}
		}
		if frame.Direction == "hub-to-node" && frame.Data["type"] == "invokefunction" && frame.Data["function_id"] == "mesh-node::workers::apply" {
			envelope, _ := frame.Data["data"].(map[string]any)
			payload, _ := envelope["payload"].(map[string]any)
			// The engine omits the already-routed namespace on worker delivery.
			if payload["namespace"] != ns || envelope["sig"] == nil {
				t.Fatalf("signed namespace bootstrap: %s", mustJSON(frame.Data))
			}
			observed["signed-apply"] = true
		}
	}
	for _, id := range []string{"configuration::ensure", "configuration::get", "signed-apply"} {
		if !observed[id] {
			t.Fatalf("missing bootstrap observation: %s", id)
		}
	}
	data, _ := json.MarshalIndent(frames, "", "  ")
	if err := os.WriteFile(filepath.Join(os.Getenv("MESH_E2E_REPORT_DIR"), "ide-"+ns+"-frames.json"), data, 0600); err != nil {
		t.Fatal(err)
	}
	for _, frame := range frames {
		if frame.Direction == "node-to-hub" {
			switch frame.Data["type"] {
			case "invokefunction", "registertrigger":
				entry := map[string]any{"type": frame.Data["type"], "function_id": frame.Data["function_id"], "namespace": frame.Data["namespace"], "trigger_type": frame.Data["trigger_type"], "config": frame.Data["config"]}
				if data, ok := frame.Data["data"].(map[string]any); ok {
					entry["configuration_id"] = data["id"]
				}
				t.Logf("IDE %s: %s", ns, mustJSON(entry))
			}
		}
	}
}
func mustJSON(v any) string { b, _ := json.Marshal(v); return string(b) }
