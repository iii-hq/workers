//go:build e2e

package e2e

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

func (f *fixture) startHarness() {
	t := f.t
	t.Helper()
	for _, w := range []struct{ name, env, ready string }{{"state", "MESH_TEST_STATE_BIN", "state::get"}, {"queue", "MESH_TEST_QUEUE_BIN", "engine::queue::list_topics"}, {"session-manager", "MESH_TEST_SESSION_BIN", "session::list"}, {"harness", "MESH_TEST_HARNESS_BIN", "harness::status"}} {
		startProcess(t, f.dir, w.name, binary(t, w.env), nil, "--url", f.engineURL)
		eventually(t, 20*time.Second, w.name+" ready", func() error {
			_, err := call(f.ctx, f.admin, f.ns, w.ready, map[string]any{"scope": "e2e", "key": "probe", "session_id": "probe"})
			return err
		})
	}
	f.startFP()
}
func (f *fixture) seedTurn(session string, allow []string, root string) {
	mustCall(f.t, f.admin, f.ns, "session::ensure", map[string]any{"session_id": session})
	record := map[string]any{"turn_id": "turn-e2e", "session_id": session, "status": "awaiting_functions", "step": 1, "turn_count": 0, "depth": 0, "created_at": 1, "updated_at": 1, "options": map[string]any{"model": "test", "max_turns": 1, "functions": map[string]any{"allow": allow}, "metadata": map[string]any{"fs_scope": map[string]any{"root": root}}}, "calls": map[string]any{}}
	mustCall(f.t, f.admin, f.ns, "state::set", map[string]any{"scope": "harness_turn", "key": session, "value": record})
}
func (f *fixture) harnessCall(session, id string, args any) json.RawMessage {
	return mustCall(f.t, f.admin, f.ns, "harness::function::trigger", map[string]any{"session_id": session, "call": map[string]any{"id": "call-e2e", "function_id": id, "arguments": args}})
}
func TestE2EFsScopeOnRemoteCoder(t *testing.T) {
	f := newFixture(t, "default")
	n := f.enroll("laptop")
	root := f.startIDE(n)
	f.startHarness()
	outside := t.TempDir()
	if err := os.WriteFile(filepath.Join(outside, "private.txt"), []byte("outside-node-root"), 0600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, "inside.txt"), []byte("inside-node-root"), 0600); err != nil {
		t.Fatal(err)
	}
	f.seedTurn("scope", []string{"laptop::coder::read-file", "fp::pipe", "fp::batch"}, t.TempDir())
	for _, mode := range []string{"direct", "pipe", "batch"} {
		t.Run(mode, func(t *testing.T) {
			for _, probe := range []struct {
				path   string
				denied bool
			}{{filepath.Join(root, "inside.txt"), false}, {filepath.Join(outside, "private.txt"), true}} {
				payload := map[string]any{"path": probe.path, "fs_scope": map[string]any{"root": outside, "grants": []string{outside}}}
				id := "laptop::coder::read-file"
				var args any = payload
				if mode != "direct" {
					key := "through"
					if mode == "batch" {
						key = "calls"
					}
					args = map[string]any{key: []any{map[string]any{"function": id, "payload": payload}}, "fs_scope": map[string]any{"root": outside, "grants": []string{outside}}}
					id = "fp::" + mode
				}
				out := f.harnessCall("scope", id, args)
				if probe.denied {
					if strings.Contains(string(out), "outside-node-root") || !strings.Contains(string(out), "C215") {
						t.Fatalf("outside root was not denied: %s", out)
					}
				} else if !strings.Contains(string(out), "inside-node-root") {
					t.Fatalf("allowed node root failed: %s", out)
				}
			}
		})
	}
	for _, frame := range n.snapshot() {
		if frame.Direction == "hub-to-node" && frame.Data["function_id"] == "coder::read-file" {
			p, _ := frame.Data["data"].(map[string]any)
			if _, ok := p["fs_scope"]; ok {
				t.Fatalf("scope reached remote coder: %s", mustJSON(p))
			}
		}
	}
}
func TestE2EHooksRefused(t *testing.T) {
	f := newFixture(t, "default")
	f.startHarness()
	n := f.enroll("evil")
	c := fakeWorker(f, n, "hook")
	var hooks, target atomic.Int32
	if err := iii.RegisterFunctionTyped(c, "hook", func(context.Context, json.RawMessage) (map[string]any, error) {
		hooks.Add(1)
		return map[string]any{"action": "continue"}, nil
	}); err != nil {
		t.Fatal(err)
	}
	if err := iii.RegisterFunctionTyped(f.admin, "target::ping", func(context.Context, json.RawMessage) (map[string]bool, error) {
		target.Add(1)
		return map[string]bool{"ok": true}, nil
	}); err != nil {
		t.Fatal(err)
	}
	for _, kind := range []string{"pre-trigger", "post-trigger"} {
		if err := c.RegisterTrigger(kind, "harness::hook::"+kind, "hook", json.RawMessage(`{}`)); err != nil {
			t.Fatal(err)
		}
		waitRegistrationRefused(t, n, kind)
	}
	f.seedTurn("hooks", []string{"target::ping"}, t.TempDir())
	out := f.harnessCall("hooks", "target::ping", map[string]any{})
	if !strings.Contains(string(out), `"is_error":false`) || target.Load() != 1 || hooks.Load() != 0 {
		t.Fatalf("hook leakage: result=%s target=%d hooks=%d", out, target.Load(), hooks.Load())
	}
}
func TestE2ERefusedRegistrationLogged(t *testing.T) {
	f := newFixture(t, "default")
	n := f.enroll("laptop")
	f.startIDE(n)
	eventually(t, 5*time.Second, "IDE receives console refusal", func() error {
		for _, frame := range n.snapshot() {
			if frame.Direction == "hub-to-node" && frame.Data["type"] == "triggerregistrationresult" && frame.Data["error"] != nil && strings.HasPrefix(fmt.Sprint(frame.Data["trigger_type"]), "console:") {
				return nil
			}
		}
		return fmt.Errorf("no refusal observed")
	})
	n.mu.Lock()
	before := n.connections
	n.mu.Unlock()
	time.Sleep(2300 * time.Millisecond)
	n.mu.Lock()
	after := n.connections
	n.mu.Unlock()
	b, err := os.ReadFile(filepath.Join(f.dir, "laptop-logs", "ide.log"))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(strings.ToLower(string(b)), "trigger registration") {
		t.Fatalf("Rust SDK did not log refused registration: %s", b)
	}
	if before != after {
		t.Fatalf("Rust SDK reconnected after refused registration: %d -> %d", before, after)
	}
	mustCall(t, f.admin, "default", "laptop::shell::exec", map[string]any{"command": "printf still-connected"})
	t.Logf("Rust SDK logs refusal and keeps its connection (%d accepted node connections including supervisor)", after)
}
