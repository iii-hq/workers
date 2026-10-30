//go:build e2e

package e2e

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

func (f *fixture) seedWorker(n *testNode, worker string, value map[string]any) {
	if f.workerSeeds == nil {
		f.workerSeeds = map[string]map[string]any{}
	}
	f.workerSeeds[n.name+"__"+worker] = value
	mustCall(f.t, f.admin, "default", "configuration::ensure", map[string]any{"id": n.name + "__" + worker, "name": worker, "description": "Owned integration fixture", "schema": map[string]any{}, "initial_value": value})
}

func (f *fixture) workerCall(id string, payload any) map[string]any {
	f.t.Helper()
	b, err := f.admin.Trigger(f.ctx, iii.TriggerRequest{FunctionID: id, Namespace: f.ns, Data: json.RawMessage(mustJSON(payload)), Timeout: 30 * time.Second})
	if err != nil {
		f.t.Fatalf("%s: %v", id, err)
	}
	var result map[string]any
	if err := json.Unmarshal(b, &result); err != nil {
		f.t.Fatalf("%s: %s: %v", id, b, err)
	}
	return result
}

func (f *fixture) waitWorker(n *testNode, worker, readyFunction string) {
	f.t.Helper()
	eventually(f.t, 30*time.Second, "real "+worker+" ready", func() error {
		_, err := call(f.ctx, f.admin, f.ns, n.name+"::"+readyFunction, map[string]any{})
		return err
	})
	name := "rbac-proxy:" + n.name + "::" + worker
	info := mustCall(f.t, f.admin, "default", "engine::workers::info", map[string]any{"name": name})
	var detail struct {
		Worker    struct{ Name, Namespace string } `json:"worker"`
		Functions []struct {
			FunctionID string `json:"function_id"`
			WorkerName string `json:"worker_name"`
			Namespace  string `json:"namespace"`
		} `json:"functions"`
	}
	if err := json.Unmarshal(info, &detail); err != nil {
		f.t.Fatal(err)
	}
	if detail.Worker.Name != name || detail.Worker.Namespace != f.ns || len(detail.Functions) == 0 {
		f.t.Fatalf("authoritative worker: %s", info)
	}
	for _, fn := range detail.Functions {
		if fn.WorkerName != name || fn.Namespace != f.ns || !strings.HasPrefix(fn.FunctionID, n.name+"::"+worker+"::") {
			f.t.Fatalf("authoritative function ownership: %s", info)
		}
	}
	stored := mustCall(f.t, f.admin, "default", "configuration::get", map[string]any{"id": n.name + "__" + worker, "raw": true})
	var entry struct {
		Value map[string]any `json:"value"`
	}
	if err := json.Unmarshal(stored, &entry); err != nil {
		f.t.Fatal(err)
	}
	for key, want := range f.workerSeeds[n.name+"__"+worker] {
		if mustJSON(entry.Value[key]) != mustJSON(want) {
			f.t.Fatalf("worker overwrote stored %s.%s: %s", worker, key, stored)
		}
	}
	if reports := os.Getenv("MESH_E2E_REPORT_DIR"); reports != "" {
		if err := os.WriteFile(filepath.Join(reports, strings.ReplaceAll(f.t.Name(), "/", "-")+"-"+worker+"-info.json"), info, 0600); err != nil {
			f.t.Fatal(err)
		}
	}
	f.t.Logf("authoritative engine worker=%s namespace=%s functions=%d; stored operator values preserved", name, f.ns, len(detail.Functions))
}

func (f *fixture) assertWorkerPolicy(n *testNode, worker string, consoleCount int) {
	f.t.Helper()
	eventually(f.t, 5*time.Second, "owned configuration and refused Console bindings", func() error {
		gets, ensures := 0, 0
		refused := map[string]bool{}
		binding, guidanceRefused := false, false
		frames := n.snapshot()
		for _, frame := range frames {
			d := frame.Data
			if frame.Direction == "node-to-hub" && d["type"] == "invokefunction" {
				p, _ := d["data"].(map[string]any)
				if p["id"] == n.name+"__"+worker {
					if d["namespace"] != "default" {
						return fmt.Errorf("configuration routing: %s", mustJSON(d))
					}
					for _, response := range frames {
						if response.Direction == "hub-to-node" && response.Data["type"] == "invocationresult" && response.Data["invocation_id"] == d["invocation_id"] {
							if response.Data["error"] != nil {
								return fmt.Errorf("owned config refused: %s", mustJSON(response.Data))
							}
							if d["function_id"] == "configuration::get" {
								gets++
							}
							if d["function_id"] == "configuration::ensure" {
								ensures++
							}
						}
					}
				}
			}
			if frame.Direction == "node-to-hub" && d["type"] == "registertrigger" && d["trigger_type"] == "configuration" {
				p, _ := d["config"].(map[string]any)
				if p["configuration_id"] == n.name+"__"+worker && d["namespace"] == f.ns {
					binding = true
				}
			}
			if frame.Direction == "hub-to-node" && d["type"] == "triggerregistrationresult" && d["trigger_type"] == "harness::hook::pre-generate" {
				failure, _ := d["error"].(map[string]any)
				guidanceRefused = failure["code"] == "REGISTRATION_DENIED"
			}
			if frame.Direction == "hub-to-node" && d["type"] == "triggerregistrationresult" && d["function_id"] == worker+"::ui-content" {
				failure, _ := d["error"].(map[string]any)
				kind, _ := d["trigger_type"].(string)
				if strings.HasPrefix(kind, "console:") && failure["code"] == "REGISTRATION_DENIED" {
					refused[kind] = true
				}
			}
		}
		if worker == "browser" && !guidanceRefused {
			return fmt.Errorf("browser guidance hook not refused")
		}
		if gets == 0 || ensures == 0 || !binding || len(refused) != consoleCount {
			return fmt.Errorf("gets=%d ensures=%d config binding=%v Console refusals=%v", gets, ensures, binding, refused)
		}
		return nil
	})
	stored := mustCall(f.t, f.admin, "default", "configuration::get", map[string]any{"id": n.name + "__" + worker, "raw": true})
	var entry struct {
		Value any `json:"value"`
	}
	if err := json.Unmarshal(stored, &entry); err != nil {
		f.t.Fatal(err)
	}
	configReceiver := worker + "::on-config-change"
	if worker == "ide" {
		configReceiver = "shell::on-config-change"
	}
	before := len(n.snapshot())
	mustCall(f.t, f.admin, "default", "configuration::set", map[string]any{"id": n.name + "__" + worker, "value": entry.Value})
	eventually(f.t, 5*time.Second, "owned configuration event delivery", func() error {
		for _, frame := range n.snapshot()[before:] {
			if frame.Direction == "hub-to-node" && frame.Data["type"] == "invokefunction" && frame.Data["function_id"] == configReceiver {
				return nil
			}
		}
		return fmt.Errorf("configuration update did not reach %s", worker)
	})
}

func (f *fixture) assertRefusedCalls(n *testNode, ids []string, code string) {
	f.t.Helper()
	eventually(f.t, 5*time.Second, "refused optional calls", func() error {
		seen := map[string]bool{}
		for _, frame := range n.snapshot() {
			d := frame.Data
			if frame.Direction != "hub-to-node" || d["type"] != "invocationresult" {
				continue
			}
			failure, _ := d["error"].(map[string]any)
			if failure["code"] == code {
				id, _ := d["function_id"].(string)
				seen[id] = true
			}
		}
		for _, id := range ids {
			if !seen[id] {
				return fmt.Errorf("missing %s refusal %s; got %v", id, code, seen)
			}
		}
		return nil
	})
}

// Only outgoing RPCs/bindings and their result status are published; enrollment,
// signed control envelopes and operation responses are outside this artifact.
func (f *fixture) observeWorker(n *testNode, worker string) {
	frames := n.snapshot()
	entries := []map[string]any{}
	for _, frame := range frames {
		d := frame.Data
		if frame.Direction != "node-to-hub" {
			continue
		}
		typeName := d["type"]
		if typeName != "invokefunction" && typeName != "registertrigger" && typeName != "registertriggertype" {
			continue
		}
		entry := map[string]any{"request": d, "result": "no acknowledgement observed"}
		for _, response := range frames {
			r := response.Data
			if response.Direction != "hub-to-node" {
				continue
			}
			matched := typeName == "invokefunction" && d["invocation_id"] != nil && r["type"] == "invocationresult" && r["invocation_id"] == d["invocation_id"]
			matched = matched || typeName == "registertrigger" && r["type"] == "triggerregistrationresult" && r["id"] == d["id"]
			if typeName == "registertrigger" && d["trigger_type"] == "configuration" && r["type"] == "invokefunction" && r["function_id"] == d["function_id"] {
				entry["result"] = "configuration event delivery observed"
			}
			if matched {
				if r["error"] != nil {
					entry["result"] = r["error"]
				} else {
					entry["result"] = "success"
				}
			}
		}
		entries = append(entries, entry)
		f.t.Logf("%s observation: type=%v function=%v namespace=%v trigger=%v config=%s result=%s", worker, d["type"], d["function_id"], d["namespace"], d["trigger_type"], mustJSON(d["config"]), mustJSON(entry["result"]))
	}
	if reports := os.Getenv("MESH_E2E_REPORT_DIR"); reports != "" {
		b, _ := json.MarshalIndent(entries, "", "  ")
		if err := os.WriteFile(filepath.Join(reports, strings.ReplaceAll(f.t.Name(), "/", "-")+"-observations.json"), b, 0600); err != nil {
			f.t.Error(err)
		}
	}
}

func (f *fixture) startXvfb() string {
	startProcess(f.t, f.dir, "xvfb", "/usr/bin/Xvfb", nil, "-displayfd", "1", "-screen", "0", "800x600x24", "-nolisten", "tcp", "-ac")
	var display string
	eventually(f.t, 5*time.Second, "owned Xvfb display", func() error {
		b, err := os.ReadFile(filepath.Join(f.dir, "xvfb.log"))
		if err != nil {
			return err
		}
		for _, line := range strings.Split(string(b), "\n") {
			n, err := strconv.Atoi(line)
			if err == nil && n >= 0 {
				display = ":" + strconv.Itoa(n)
				return nil
			}
		}
		return fmt.Errorf("Xvfb has not reported a display")
	})
	return display
}
