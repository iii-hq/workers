//go:build e2e

package e2e

import (
	"context"
	"encoding/json"
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
	return mustCall(f.t, f.admin, f.ns, "harness::function::call", map[string]any{"session_id": session, "call": map[string]any{"id": "call-e2e", "function_id": id, "arguments": args}})
}
func TestE2EHooksRefused(t *testing.T) {
	f := newFixture(t, "default")
	f.startHarness()
	n := f.enroll("evil")
	c := fakeWorker(f, n, "hook")
	var hooks, target atomic.Int32
	if _, err := iii.RegisterFunctionTyped(c, "hook", func(context.Context, json.RawMessage) (map[string]any, error) {
		hooks.Add(1)
		return map[string]any{"action": "continue"}, nil
	}); err != nil {
		t.Fatal(err)
	}
	if _, err := iii.RegisterFunctionTyped(f.admin, "target::ping", func(context.Context, json.RawMessage) (map[string]bool, error) {
		target.Add(1)
		return map[string]bool{"ok": true}, nil
	}); err != nil {
		t.Fatal(err)
	}
	for _, kind := range []string{"on-pre-call", "on-post-call"} {
		observe(t, c, iii.Source{FunctionID: "harness::hook::" + kind, Namespace: f.ns}, "hook", kind, false)
	}
	f.seedTurn("hooks", []string{"target::ping"}, t.TempDir())
	out := f.harnessCall("hooks", "target::ping", map[string]any{})
	if !strings.Contains(string(out), `"is_error":false`) || target.Load() != 1 || hooks.Load() != 0 {
		t.Fatalf("hook leakage: result=%s target=%d hooks=%d", out, target.Load(), hooks.Load())
	}
}
