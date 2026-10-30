//go:build e2e

package e2e

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
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
	defer f.observeWorker(n, "ide")
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
	f.assertWorkerPolicy(n, "ide", 3)
	f.assertRefusedCalls(n, []string{"configuration::get"}, "invocation_failed")
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

func TestE2EBrowserOnNode(t *testing.T) {
	for _, ns := range []string{"default", "project"} {
		t.Run(ns, func(t *testing.T) {
			f := newFixture(t, ns)
			n := f.enroll("laptop")
			page := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				fmt.Fprint(w, "<!doctype html><title>Mesh browser fixture</title><h1>Owned browser page</h1>")
			}))
			t.Cleanup(page.Close)
			f.seedWorker(n, "browser", map[string]any{"executable": binary(t, "MESH_TEST_CHROMIUM_BIN"), "data_dir": filepath.Join(f.dir, "browser"), "headless": true, "frame_density": 1, "scrapling": map[string]any{"adaptive_storage_path": filepath.Join(f.dir, "adaptive.db")}})
			s := f.startWorker(n, "browser", "0.2.33", "MESH_TEST_BROWSER_BIN")
			f.waitWorker(n, "browser", "browser::sessions::list")
			defer f.observeWorker(n, "browser")
			started := f.workerCall("laptop::browser::sessions::start", map[string]any{"url": page.URL, "incognito": true, "preview": false})
			id, _ := started["session_id"].(string)
			if id == "" || started["headless"] != true || started["timed_out"] != false || started["error"] != nil {
				t.Fatalf("browser start: %s", mustJSON(started))
			}
			snapshot := f.workerCall("laptop::browser::snapshot", map[string]any{"session_id": id})
			if snapshot["title"] != "Mesh browser fixture" || !strings.Contains(fmt.Sprint(snapshot["tree"]), "Owned browser page") {
				t.Fatalf("real Chromium page: %s", mustJSON(snapshot))
			}
			stopped := f.workerCall("laptop::browser::sessions::stop", map[string]any{"session_id": id})
			if stopped["was_running"] != true {
				t.Fatalf("browser stop: %s", mustJSON(stopped))
			}
			f.assertWorkerPolicy(n, "browser", 2)
			if status := s.Status(); len(status) != 1 || status[0].Restarts != 0 || status[0].State != "running" {
				t.Fatalf("worker did not survive optional refusals: %+v", status)
			}
			t.Log("real Chromium start/snapshot/stop completed; supervisor running, restarts=0")
		})
	}
}

func TestE2ETmuxOnNode(t *testing.T) {
	f := newFixture(t, "project")
	n := f.enroll("laptop")
	socket := filepath.Join(f.dir, "tmux.sock")
	t.Cleanup(func() { _ = exec.Command("/usr/bin/tmux", "-S", socket, "kill-server").Run() })
	if out, err := exec.Command("/usr/bin/tmux", "-f", "/dev/null", "-S", socket, "new-session", "-d", "-s", "baseline", "sleep 120").CombinedOutput(); err != nil {
		t.Fatalf("private tmux server: %v: %s", err, out)
	}
	f.seedWorker(n, "tmux", map[string]any{"tmux_binary": "/usr/bin/tmux", "socket_path": socket})
	s := f.startWorker(n, "tmux", "0.1.3", "MESH_TEST_TMUX_BIN")
	f.waitWorker(n, "tmux", "tmux::status")
	defer f.observeWorker(n, "tmux")
	created := f.workerCall("laptop::tmux::sessions::create", map[string]any{"name": "mesh-fixture", "cwd": f.dir, "command": "printf mesh-tmux-owned; exec sleep 120"})
	pane, _ := created["pane_id"].(string)
	if pane == "" {
		t.Fatalf("tmux create: %s", mustJSON(created))
	}
	eventually(t, 5*time.Second, "owned tmux pane output", func() error {
		out, err := call(f.ctx, f.admin, f.ns, "laptop::tmux::panes::capture", map[string]any{"pane": pane})
		if err != nil {
			return err
		}
		if !strings.Contains(string(out), "mesh-tmux-owned") {
			return fmt.Errorf("capture: %s", out)
		}
		return nil
	})
	killed := f.workerCall("laptop::tmux::sessions::kill", map[string]any{"session": created["session_id"]})
	if killed["killed"] != true {
		t.Fatalf("tmux kill: %s", mustJSON(killed))
	}
	f.assertWorkerPolicy(n, "tmux", 3)
	if status := s.Status(); len(status) != 1 || status[0].Restarts != 0 || status[0].State != "running" {
		t.Fatalf("tmux after refusals: %+v", status)
	}
	t.Log("real tmux create/capture/kill completed on private socket; supervisor restarts=0")
}

func TestE2EComputerOnNode(t *testing.T) {
	f := newFixture(t, "project")
	n := f.enroll("laptop")
	f.seedWorker(n, "computer", map[string]any{})
	s := f.startWorker(n, "computer", "0.1.24", "MESH_TEST_COMPUTER_BIN")
	f.waitWorker(n, "computer", "computer::sessions::list")
	defer f.observeWorker(n, "computer")
	_, err := call(f.ctx, f.admin, f.ns, "laptop::computer::sessions::start", map[string]any{"native": true})
	if err == nil || !strings.Contains(err.Error(), "no X11 or Wayland display") {
		t.Fatalf("supervised no-display result: %v", err)
	}
	t.Logf("supervised computer native start (clean environment): %v", err)
	f.assertWorkerPolicy(n, "computer", 2)
	if status := s.Status(); len(status) != 1 || status[0].Restarts != 0 || status[0].State != "running" {
		t.Fatalf("computer after refusals: %+v", status)
	}
	s.Close()
	eventually(t, 5*time.Second, "supervised computer disconnect", func() error {
		_, err := call(f.ctx, f.admin, f.ns, "laptop::computer::sessions::list", map[string]any{})
		if err == nil {
			return fmt.Errorf("supervised worker still registered")
		}
		return nil
	})
	display := f.startXvfb()
	startProcess(t, f.dir, "computer-xvfb", binary(t, "MESH_TEST_COMPUTER_BIN"), []string{"III_URL=" + n.url, "III_NAMESPACE=" + f.ns, "III_CONFIG_NAME=laptop__computer", "DISPLAY=" + display, "WAYLAND_DISPLAY=", "DBUS_SESSION_BUS_ADDRESS=", "XAUTHORITY="})
	f.waitWorker(n, "computer", "computer::sessions::list")
	started := f.workerCall("laptop::computer::sessions::start", map[string]any{"native": true})
	id, _ := started["session_id"].(string)
	if id == "" || started["endpoint"] != "native" {
		t.Fatalf("computer start: %s", mustJSON(started))
	}
	shot := f.workerCall("laptop::computer::screenshot", map[string]any{"session_id": id})
	details, _ := shot["details"].(map[string]any)
	if details["width"] != float64(800) || details["height"] != float64(600) {
		t.Fatalf("private Xvfb dimensions: %s", mustJSON(details))
	}
	content, _ := shot["content"].([]any)
	if len(content) == 0 {
		t.Fatal("missing captured image")
	}
	block, _ := content[0].(map[string]any)
	data, _ := block["data"].(string)
	decoded, err := base64.StdEncoding.DecodeString(data)
	if err != nil || len(decoded) < 100 {
		t.Fatalf("invalid captured image: %v size=%d", err, len(decoded))
	}
	f.workerCall("laptop::computer::sessions::stop", map[string]any{"session_id": id})
	f.assertRefusedCalls(n, []string{"state::list", "state::set", "state::delete"}, "FORBIDDEN")
	t.Logf("supplemental direct binary through gateway: private Xvfb %s start/screenshot/stop; capture bytes=%d; shared persistence refused", display, len(decoded))
}
