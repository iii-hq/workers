//go:build e2e

package e2e

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
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
	b, err := f.admin.Call(f.ctx, iii.CallRequest{FunctionID: id, Namespace: f.ns, Payload: json.RawMessage(mustJSON(payload)), Timeout: 30 * time.Second})
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

func (f *fixture) assertWorkerPolicy(n *testNode, worker string, assetCount int) {
	f.t.Helper()
	eventually(f.t, 5*time.Second, "owned configuration and refused ADE bindings", func() error {
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
						if response.Connection == frame.Connection && response.Direction == "hub-to-node" && response.Data["type"] == "invocationresult" && response.Data["invocation_id"] == d["invocation_id"] {
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
			if frame.Direction != "node-to-hub" || d["type"] != "observe" {
				continue
			}
			source, _ := d["source"].(map[string]any)
			target, _ := d["target"].(map[string]any)
			filters, _ := source["filters"].(map[string]any)
			idFilter, _ := filters["input.id"].(map[string]any)
			for _, response := range frames {
				r := response.Data
				if response.Connection != frame.Connection || response.Direction != "hub-to-node" || r["type"] != "observeresult" || r["id"] != d["id"] {
					continue
				}
				if source["function_id"] == "configuration::on-updated" && source["namespace"] == "default" && idFilter["$eq"] == n.name+"__"+worker && target["namespace"] == f.ns && r["state"] == "active" && r["error"] == nil {
					binding = true
				}
				failure, _ := r["error"].(map[string]any)
				if failure["code"] != "FORBIDDEN" {
					continue
				}
				if source["function_id"] == "harness::hook::on-pre-generate" {
					guidanceRefused = true
				}
				if source["function_id"] == "ade::ui::asset" && target["function_id"] == worker+"::ui-content" {
					args, _ := source["args"].(map[string]any)
					refused[fmt.Sprint(args["path"])] = true
				}
			}
		}
		if worker == "browser" && !guidanceRefused {
			return fmt.Errorf("browser guidance observer not refused")
		}
		if gets == 0 || ensures == 0 || !binding || len(refused) != assetCount {
			return fmt.Errorf("gets=%d ensures=%d config observer=%v ADE refusals=%v", gets, ensures, binding, refused)
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
	before := len(n.snapshot())
	mustCall(f.t, f.admin, "default", "configuration::set", map[string]any{"id": n.name + "__" + worker, "value": entry.Value})
	eventually(f.t, 5*time.Second, "owned configuration delivery and hot reload refetch", func() error {
		entries := workerObservations(n.snapshot(), n.name+"__"+worker)
		delivered := false
		for _, e := range entries {
			if e["result"] == "configuration event delivery observed" {
				delivered = true
			}
		}
		refetch := false
		for _, frame := range n.snapshot()[before:] {
			p, _ := frame.Data["data"].(map[string]any)
			if frame.Direction == "node-to-hub" && frame.Data["type"] == "invokefunction" && frame.Data["function_id"] == "configuration::get" && p["id"] == n.name+"__"+worker {
				refetch = true
			}
		}
		if !delivered || !refetch {
			return fmt.Errorf("delivery=%v config-refetch=%v", delivered, refetch)
		}
		return nil
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

func workerObservations(frames []observedFrame, configurationID string) []map[string]any {
	entries := []map[string]any{}
	for i, frame := range frames {
		d := frame.Data
		if frame.Direction != "node-to-hub" || (d["type"] != "invokefunction" && d["type"] != "observe") {
			continue
		}
		entry := map[string]any{"connection": frame.Connection, "request": d, "result": "no acknowledgement observed"}
		source, _ := d["source"].(map[string]any)
		target, _ := d["target"].(map[string]any)
		filters, _ := source["filters"].(map[string]any)
		idFilter, _ := filters["input.id"].(map[string]any)
		for _, response := range frames[i+1:] {
			r := response.Data
			if response.Direction != "hub-to-node" || frame.Connection <= 0 || response.Connection != frame.Connection {
				continue
			}
			matched := d["type"] == "invokefunction" && d["invocation_id"] != nil && r["type"] == "invocationresult" && r["invocation_id"] == d["invocation_id"]
			matched = matched || (d["type"] == "observe" && r["type"] == "observeresult" && r["id"] == d["id"])
			activation, _ := r["activation"].(map[string]any)
			event, _ := r["data"].(map[string]any)
			if d["type"] == "observe" && source["function_id"] == "configuration::on-updated" && configurationID != "" && idFilter["$eq"] == configurationID && d["id"] != nil && activation["observer_id"] == d["id"] && r["type"] == "invokefunction" && target["function_id"] != "" && r["function_id"] == target["function_id"] && event["event_type"] == "configuration:updated" && event["id"] == configurationID {
				entry["result"] = "configuration event delivery observed"
			}
			if matched && entry["result"] != "configuration event delivery observed" {
				if r["error"] != nil {
					entry["result"] = r["error"]
				} else if d["type"] == "invokefunction" || r["state"] == "active" {
					entry["result"] = "success"
				}
			}
		}
		entries = append(entries, entry)
	}
	return entries
}

// Only outgoing RPCs/bindings and their result status are published; enrollment,
// signed control envelopes and operation responses are outside this artifact.
func (f *fixture) observeWorker(n *testNode, worker string) {
	entries := workerObservations(n.snapshot(), n.name+"__"+worker)
	for _, entry := range entries {
		d := entry["request"].(map[string]any)
		f.t.Logf("%s observation: type=%v function=%v namespace=%v source=%s target=%s result=%s", worker, d["type"], d["function_id"], d["namespace"], mustJSON(d["source"]), mustJSON(d["target"]), mustJSON(entry["result"]))
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

func TestWorkerObservationsConfigurationDelivery(t *testing.T) {
	registration := func(connection int, id, observer string) observedFrame {
		return observedFrame{Connection: connection, Direction: "node-to-hub", Data: map[string]any{"type": "observe", "id": observer, "source": map[string]any{"function_id": "configuration::on-updated", "filters": map[string]any{"input.id": map[string]any{"$eq": id}}}, "target": map[string]any{"function_id": "computer::on-config-change"}}}
	}
	delivery := func(connection int, id, observer string) observedFrame {
		return observedFrame{Connection: connection, Direction: "hub-to-node", Data: map[string]any{"type": "invokefunction", "function_id": "computer::on-config-change", "activation": map[string]any{"observer_id": observer}, "data": map[string]any{"event_type": "configuration:updated", "id": id}}}
	}
	const owned = "laptop__computer"
	for _, tc := range []struct {
		name   string
		frames []observedFrame
		want   []bool
	}{
		{"current delivery", []observedFrame{registration(1, owned, "one"), delivery(1, owned, "one")}, []bool{true}},
		{"historical delivery before new process", []observedFrame{registration(1, owned, "one"), delivery(1, owned, "one"), registration(2, owned, "two")}, []bool{true, false}},
		{"historical delivery on same connection", []observedFrame{delivery(1, owned, "one"), registration(1, owned, "one")}, []bool{false}},
		{"old connection delivers later", []observedFrame{registration(1, owned, "one"), registration(2, owned, "two"), delivery(1, owned, "one")}, []bool{true, false}},
		{"each process own delivery", []observedFrame{registration(1, owned, "one"), delivery(1, owned, "one"), registration(2, owned, "two"), delivery(2, owned, "two")}, []bool{true, true}},
		{"wrong configuration", []observedFrame{registration(1, owned, "one"), delivery(1, "desktop__computer", "one")}, []bool{false}},
		{"nonowned registration", []observedFrame{registration(1, "desktop__computer", "one"), delivery(1, "desktop__computer", "one")}, []bool{false}},
		{"missing connection", []observedFrame{registration(0, owned, "one"), delivery(0, owned, "one")}, []bool{false}},
		{"different observer same target", []observedFrame{registration(1, owned, "one"), delivery(1, owned, "two")}, []bool{false}},
		{"later observer same target", []observedFrame{registration(1, owned, "one"), registration(1, owned, "two"), delivery(1, owned, "two")}, []bool{false, true}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			entries := workerObservations(tc.frames, owned)
			if len(entries) != len(tc.want) {
				t.Fatalf("entries=%d want=%d", len(entries), len(tc.want))
			}
			for i, e := range entries {
				want := "no acknowledgement observed"
				if tc.want[i] {
					want = "configuration event delivery observed"
				}
				if e["result"] != want {
					t.Errorf("registration %d result=%v want=%s", i, e["result"], want)
				}
			}
		})
	}
}
