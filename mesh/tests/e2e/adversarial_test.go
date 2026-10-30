//go:build e2e

package e2e

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/coder/websocket"
	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/hub"
)

func fakeWorker(f *fixture, n *testNode, name string) *iii.Client {
	c := connect(f.t, f.ctx, n.url, name, f.ns)
	if err := iii.RegisterFunctionTyped(c, "echo", func(context.Context, json.RawMessage) (map[string]string, error) {
		return map[string]string{"node": n.name}, nil
	}); err != nil {
		f.t.Fatal(err)
	}
	eventually(f.t, 5*time.Second, "fake echo registration", func() error {
		_, err := call(f.ctx, f.admin, f.ns, n.name+"::echo", map[string]any{"node": n.name})
		return err
	})
	return c
}
func refused(t *testing.T, c *iii.Client, ns, id string, p any, reason string) {
	t.Helper()
	out, err := call(t.Context(), c, ns, id, p)
	if err == nil || !strings.Contains(strings.ToLower(err.Error()), strings.ToLower(reason)) {
		t.Fatalf("%s: got %s, %v; want refusal containing %q", id, out, err, reason)
	}
}
func configEntry(id string, value any) map[string]any {
	return map[string]any{"id": id, "name": "test", "description": "test", "schema": map[string]any{}, "initial_value": value}
}
func TestE2ETemplatesRefused(t *testing.T) {
	f := newFixture(t, "default")
	n := f.enroll("evil")
	c := fakeWorker(f, n, "probe")
	for _, field := range []string{"value", "initial_value"} {
		p := configEntry("evil__probe", map[string]any{})
		p[field] = map[string]any{"secret": "${OPENAI_API_KEY}"}
		refused(t, c, "default", "configuration::ensure", p, "templates")
	}
	mustCall(t, f.admin, "default", "configuration::ensure", configEntry("evil__operator", map[string]any{"raw": "${OPENAI_API_KEY}"}))
	result := mustCall(t, c, "default", "configuration::get", map[string]any{"id": "evil__operator"})
	if !strings.Contains(string(result), "${OPENAI_API_KEY}") {
		t.Fatalf("expanded operator value: %s", result)
	}
	refused(t, c, "default", "configuration::get", map[string]any{"id": "mesh"}, "outside node scope")
}
func TestE2EMiddlewareNeedsSessionKey(t *testing.T) {
	f := newFixture(t, "default")
	refused(t, f.admin, "default", "mesh::middleware", map[string]any{"function_id": "configuration::get", "payload": map[string]any{"id": "mesh"}}, "invalid mesh session")
}
func TestE2ELoopbackWithoutTicket(t *testing.T) {
	f := newFixture(t, "default")
	ctx, cancel := context.WithTimeout(f.ctx, 3*time.Second)
	defer cancel()
	conn, _, err := websocket.Dial(ctx, f.proxyURL, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer conn.CloseNow()
	_, message, err := conn.Read(ctx)
	if err != nil || !strings.Contains(string(message), `"type":"error"`) || !strings.Contains(string(message), "AUTH") {
		t.Fatalf("ticketless proxy did not reject: %s, %v", message, err)
	}
	_, _, err = conn.Read(ctx)
	if websocket.CloseStatus(err) == -1 {
		t.Fatalf("ticketless connection not closed: %v", err)
	}

}
func TestE2ENodeRegistersMeshIDGetsPrefix(t *testing.T) {
	f := newFixture(t, "default")
	n := f.enroll("evil")
	c := fakeWorker(f, n, "mesh")
	if err := iii.RegisterFunctionTyped(c, "mesh::status", func(context.Context, json.RawMessage) (map[string]bool, error) {
		return map[string]bool{"evil": true}, nil
	}); err != nil {
		t.Fatal(err)
	}
	eventually(t, 5*time.Second, "prefixed mesh registration", func() error {
		b, err := call(f.ctx, f.admin, "default", "evil::mesh::status", map[string]any{})
		if err == nil && !strings.Contains(string(b), `"evil":true`) {
			return fmt.Errorf("unexpected %s", b)
		}
		return err
	})
	original := mustCall(t, f.admin, "default", "mesh::status", map[string]any{})
	if strings.Contains(string(original), `"evil":true`) {
		t.Fatal("node replaced hub function")
	}
}
func TestE2ERevocationWithin5s(t *testing.T) {
	f := newFixture(t, "default")
	var calls atomic.Int32
	if err := iii.RegisterFunctionTyped(f.admin, "target::allowed", func(context.Context, json.RawMessage) (map[string]bool, error) {
		calls.Add(1)
		return map[string]bool{"ok": true}, nil
	}); err != nil {
		t.Fatal(err)
	}
	f.config.Nodes["evil"] = hub.NodeConfig{Workers: []string{}, Allow: []string{"target::allowed"}}
	f.reload()
	n := f.enroll("evil")
	c := fakeWorker(f, n, "echo")
	mustCall(t, c, "default", "target::allowed", map[string]any{})
	start := time.Now()
	f.config.Nodes["evil"] = hub.NodeConfig{Workers: []string{}, Allow: []string{}}
	f.reload()
	deadline := start.Add(5 * time.Second)
	ctx, cancel := context.WithDeadline(f.ctx, deadline)
	defer cancel()
	if err := waitForPolicyRefusal(ctx, func(callCtx context.Context) error {
		_, err := call(callCtx, c, "default", "target::allowed", map[string]any{})
		return err
	}); err != nil {
		t.Fatalf("existing worker grant not revoked within 5s: %v", err)
	}
	enforcedAt := time.Now()
	if !enforcedAt.Before(deadline) {
		t.Fatalf("policy refusal completed after 5s: %s", enforcedAt.Sub(start))
	}
	baseline := calls.Load()
	refused(t, c, "default", "target::allowed", map[string]any{}, "forbidden")
	if calls.Load() != baseline {
		t.Fatal("revoked invocation reached target")
	}
	t.Logf("revocation enforced in %s", enforcedAt.Sub(start))
}
func (f *fixture) startFP() {
	startProcess(f.t, f.dir, "fp", binary(f.t, "MESH_TEST_FP_BIN"), nil, "--url", f.engineURL)
	eventually(f.t, 15*time.Second, "fp ready", func() error {
		_, err := call(f.ctx, f.admin, f.ns, "fp::batch", map[string]any{"calls": []any{map[string]any{"function": "mesh::status", "payload": map[string]any{}}}})
		return err
	})
}
func TestE2ESupervisorUnreachable(t *testing.T) {
	f := newFixture(t, "default")
	n := f.enroll("laptop")
	f.startNode(n, false)
	f.startFP()
	eventually(t, 5*time.Second, "signed supervisor registered", func() error {
		_, err := call(f.ctx, f.admin, "default", "laptop::mesh-node::workers::apply", map[string]any{})
		if err != nil && strings.Contains(err.Error(), "unsigned") {
			return nil
		}
		return fmt.Errorf("got %v", err)
	})
	refused(t, f.admin, "default", "laptop::mesh-node::workers::apply", map[string]any{}, "unsigned")
	for _, kind := range []string{"pipe", "batch"} {
		key := "through"
		if kind == "batch" {
			key = "calls"
		}
		b, err := call(f.ctx, f.admin, "default", "fp::"+kind, map[string]any{key: []any{map[string]any{"function": "laptop::mesh-node::workers::apply", "payload": map[string]any{}}}})
		rejection := fmt.Sprintf("%s %v", b, err)
		if !strings.Contains(rejection, "bus-internal functions are not supported") {
			t.Fatalf("%s did not refuse supervisor: %s", kind, rejection)
		}

	}
	var envelope any
	eventually(t, 5*time.Second, "hub signed apply observed", func() error {
		for _, frame := range n.snapshot() {
			if frame.Direction == "hub-to-node" && frame.Data["type"] == "invokefunction" && frame.Data["function_id"] == "mesh-node::workers::apply" {
				payload, _ := frame.Data["data"].(map[string]any)
				if signature, ok := payload["sig"].(string); !ok || signature == "" {
					continue
				}
				envelope = payload
				return nil
			}
		}
		return fmt.Errorf("no signed apply")
	})
	refused(t, f.admin, "default", "laptop::mesh-node::workers::apply", envelope, "replayed")
}

func waitRegistrationRefused(t *testing.T, n *testNode, id string) {
	t.Helper()
	eventually(t, 5*time.Second, "registration refusal "+id, func() error {
		for _, frame := range n.snapshot() {
			if frame.Direction == "hub-to-node" && frame.Data["type"] == "triggerregistrationresult" && frame.Data["id"] == id && frame.Data["error"] != nil {
				return nil
			}
		}
		return fmt.Errorf("no error acknowledgement")
	})
}
func TestE2EConditionCallbackRefused(t *testing.T) {
	f := newFixture(t, "default")
	n := f.enroll("evil")
	c := fakeWorker(f, n, "echo")
	var called atomic.Int32
	if err := iii.RegisterFunctionTyped(f.admin, "outside::callback", func(context.Context, json.RawMessage) (bool, error) { called.Add(1); return true, nil }); err != nil {
		t.Fatal(err)
	}
	mustCall(t, c, "default", "configuration::ensure", configEntry("evil__probe", map[string]any{}))
	for i, condition := range []string{"outside::callback", "echo"} {
		id := fmt.Sprintf("condition-%d", i)
		cfg := json.RawMessage(mustJSON(map[string]any{"configuration_id": "evil__probe", "condition_function_id": condition}))
		if err := c.RegisterTrigger(id, "configuration", "echo", cfg); err != nil {
			t.Fatal(err)
		}
		waitRegistrationRefused(t, n, id)
	}
	var delivered atomic.Int32
	if err := iii.RegisterFunctionTyped(c, "on-change", func(context.Context, json.RawMessage) (json.RawMessage, error) {
		delivered.Add(1)
		return json.RawMessage(`{}`), nil
	}); err != nil {
		t.Fatal(err)
	}
	if err := c.RegisterTrigger("positive-config", "configuration", "on-change", json.RawMessage(`{"configuration_id":"evil__probe","event_types":["configuration:updated"]}`)); err != nil {
		t.Fatal(err)
	}
	eventually(t, 5*time.Second, "positive config binding accepted", func() error {
		b, err := call(f.ctx, f.admin, "default", "engine::registered-triggers::list", map[string]any{})
		if err != nil {
			return err
		}
		if strings.Contains(string(b), "positive-config") {
			return nil
		}
		return fmt.Errorf("binding absent: %s", b)
	})

	mustCall(t, c, "default", "configuration::set", map[string]any{"id": "evil__probe", "value": map[string]any{"changed": true}})
	eventually(t, 5*time.Second, "configuration update delivered", func() error {
		if delivered.Load() > 0 {
			return nil
		}
		return fmt.Errorf("positive config callback did not run")
	})
	if called.Load() != 0 {
		t.Fatal("ungranted condition ran")
	}
	if err := c.RegisterTrigger("foreign-config", "configuration", "echo", json.RawMessage(`{"configuration_id":"mesh"}`)); err != nil {
		t.Fatal(err)
	}
	waitRegistrationRefused(t, n, "foreign-config")
}
func TestE2ENameSquatting(t *testing.T) {
	f := newFixture(t, "default")
	evil := f.enroll("evil")
	fakeWorker(f, evil, "ide")
	before := mustCall(t, f.admin, "default", "engine::workers::list", map[string]any{})
	if !strings.Contains(string(before), "rbac-proxy:evil::ide") {
		t.Fatalf("unconfined evil worker name: %s", before)
	}
	mac := f.enroll("mac")
	fakeWorker(f, mac, "ide")
	workers := mustCall(t, f.admin, "default", "engine::workers::list", map[string]any{})
	for _, name := range []string{"rbac-proxy:evil::ide", "rbac-proxy:mac::ide"} {
		if !strings.Contains(string(workers), name) {
			t.Fatalf("missing %s: %s", name, workers)
		}
	}
	for _, name := range []string{"evil", "mac"} {
		b := mustCall(t, f.admin, "default", name+"::echo", map[string]any{"probe": "same-payload"})
		var reply struct{ Node string }
		if err := json.Unmarshal(b, &reply); err != nil || reply.Node != name {
			t.Fatalf("displaced %s: %s", name, b)
		}
	}
}

type triggerRecorder struct{ registered chan iii.TriggerConfig }

func (r triggerRecorder) RegisterTrigger(_ context.Context, cfg iii.TriggerConfig) error {
	r.registered <- cfg
	return nil
}
func (r triggerRecorder) UnregisterTrigger(context.Context, iii.TriggerConfig) error { return nil }
func TestE2ETwoNodesSameWorkerName(t *testing.T) {
	f := newFixture(t, "default")
	for _, name := range []string{"alpha", "beta"} {
		n := f.enroll(name)
		c := fakeWorker(f, n, "echo")
		received := make(chan iii.TriggerConfig, 4)
		if err := c.RegisterTriggerType("echo::event", "Echo event", triggerRecorder{received}); err != nil {
			t.Fatal(err)
		}
		if err := c.RegisterTrigger("binding", "echo::event", "echo", json.RawMessage(`{}`)); err != nil {
			t.Fatal(err)
		}
		select {
		case cfg := <-received:
			if cfg.ID != "binding" || cfg.FunctionID != "echo" {
				t.Fatalf("type delivery not unprefixed: %+v", cfg)
			}
		case <-time.After(5 * time.Second):
			t.Fatal("own trigger binding never reached fake worker")
		}
	}
	for _, name := range []string{"alpha", "beta"} {
		out := mustCall(t, f.admin, "default", name+"::echo", map[string]any{"probe": "same-payload"})
		var reply struct{ Node string }
		if err := json.Unmarshal(out, &reply); err != nil || reply.Node != name {
			t.Fatalf("echo displaced: %s", out)
		}
	}
	workers := mustCall(t, f.admin, "default", "engine::workers::list", map[string]any{})
	for _, name := range []string{"alpha", "beta"} {
		if !strings.Contains(string(workers), "rbac-proxy:"+name+"::echo") {
			t.Fatalf("missing %s worker: %s", name, workers)
		}
	}
}
