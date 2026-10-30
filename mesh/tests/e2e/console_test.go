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

// Engine metadata is authoritative here: the proxy rewrites the upstream worker name.
func TestE2EConsoleNodeConfigurationDiscovery(t *testing.T) {
	for _, ns := range []string{"default", "project"} {
		t.Run(ns, func(t *testing.T) {
			f := newFixture(t, ns)
			n := f.enroll("laptop")
			f.startIDE(n)
			name := "rbac-proxy:laptop::ide"
			workers := mustCall(t, f.admin, "default", "engine::workers::list", map[string]any{})
			info := mustCall(t, f.admin, "default", "engine::workers::info", map[string]any{"name": name})
			var catalog struct {
				Workers []struct{ ID, Name, Namespace, Status string } `json:"workers"`
			}
			var detail struct {
				Worker    struct{ ID, Name, Namespace string } `json:"worker"`
				Functions []struct {
					FunctionID string `json:"function_id"`
					WorkerName string `json:"worker_name"`
					Namespace  string `json:"namespace"`
				} `json:"functions"`
			}
			if err := json.Unmarshal(workers, &catalog); err != nil {
				t.Fatal(err)
			}
			if err := json.Unmarshal(info, &detail); err != nil {
				t.Fatal(err)
			}
			live := 0
			for _, worker := range catalog.Workers {
				if worker.Name == name && (worker.Status == "connected" || worker.Status == "available") {
					live++
					if detail.Worker.ID != worker.ID || worker.Namespace != ns {
						t.Fatalf("worker identity mismatch: %s", info)
					}
				}
			}
			if live != 1 {
				t.Fatalf("exact live workers=%d: %s", live, workers)
			}
			identities := 0
			for _, fn := range detail.Functions {
				if !strings.HasSuffix(fn.FunctionID, "::configuration-id") {
					continue
				}
				identities++
				if fn.WorkerName != name || fn.Namespace != ns || detail.Worker.Namespace != ns {
					t.Fatalf("Console identity verification fails: %s", info)
				}
				identity := mustCall(t, f.admin, ns, fn.FunctionID, map[string]any{})
				if string(identity) != `{"id":"laptop__ide"}` {
					t.Fatalf("identity: %s", identity)
				}
				schema := mustCall(t, f.admin, "default", "configuration::schema", map[string]any{"id": "laptop__ide"})
				var entry struct {
					ID       string `json:"id"`
					Metadata struct {
						UIForm string `json:"ui_form"`
					} `json:"metadata"`
				}
				if err := json.Unmarshal(schema, &entry); err != nil {
					t.Fatal(err)
				}
				if entry.ID != "laptop__ide" || entry.Metadata.UIForm != "ide" {
					t.Fatalf("schema family: %s", schema)
				}
				t.Logf("Console discovery: worker=%s namespace=%s function=%s identity=%s family=%s", name, ns, fn.FunctionID, identity, entry.Metadata.UIForm)
			}
			if identities != 1 {
				t.Fatalf("Console identity count=%d: %s", identities, info)
			}
			refusals := map[string]map[string]any{}
			eventually(t, 5*time.Second, "remote UI registrations refused", func() error {
				for _, frame := range n.snapshot() {
					if frame.Direction != "hub-to-node" || frame.Data["type"] != "triggerregistrationresult" || frame.Data["function_id"] != "ide::ui-content" {
						continue
					}
					kind, _ := frame.Data["trigger_type"].(string)
					if kind != "console:script" && kind != "console:style" && kind != "console:module" {
						continue
					}
					failure, _ := frame.Data["error"].(map[string]any)
					if failure["code"] == "REGISTRATION_DENIED" {
						refusals[kind] = frame.Data
					}
				}
				if len(refusals) != 3 {
					return fmt.Errorf("refusals=%v", refusals)
				}
				return nil
			})
			rawRefusals, _ := json.MarshalIndent(refusals, "", "  ")
			if err := os.WriteFile(filepath.Join(os.Getenv("MESH_E2E_REPORT_DIR"), "console-node-"+ns+"-ui-refusals.json"), rawRefusals, 0600); err != nil {
				t.Fatal(err)
			}
			t.Log("Console form delivery: remote module/script/style registrations rejected by Mesh policy; identity/schema discovery works, node-only form assets require follow-up")
			if err := os.WriteFile(filepath.Join(os.Getenv("MESH_E2E_REPORT_DIR"), "console-node-"+ns+"-info.json"), info, 0600); err != nil {
				t.Fatal(err)
			}
			t.Logf("Actual engine worker info: %s", info)
		})
	}
}
