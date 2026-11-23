//go:build e2e

package e2e

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"slices"
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	meshclient "github.com/iii-hq/workers/mesh/internal/client"
)

// What a harness on the calling machine stamps on a shell call.
const stamped = `{"command":"hostname","fs_scope":{"root":"/caller","grants":["/caller/x"],"boundary":"workspace"},"caller_fs_root":"/caller","caller_session_id":"s-caller"}`

// Two machines share a worker named shell. An agent calls shell::exec by namespace, from the hub
// and from each machine, and reaches the machine the namespace names.
func TestE2ETwoMachinesShareShell(t *testing.T) {
	f := newFixture(t, "project")
	alpha, beta := f.machine("alpha"), f.machine("beta")
	for _, m := range []*machine{alpha, beta} {
		shell := connect(t, f.ctx, m.engineURL, "shell", m.local)
		if _, err := iii.RegisterFunctionTyped(shell, "shell::exec", func(_ context.Context, got json.RawMessage) (map[string]any, error) {
			return map[string]any{"node": m.name, "got": got}, nil
		}); err != nil {
			t.Fatal(err)
		}
		eventually(t, 15*time.Second, m.name+" shares shell", func() error {
			if err := m.bridge.Sync(f.ctx, []string{"shell"}); err != nil {
				return err
			}
			if s := m.bridge.Status(); len(s) != 1 || s[0].State != "shared" || !slices.Equal(s[0].Functions, []string{"shell::exec"}) {
				return fmt.Errorf("status %+v", s)
			}
			return nil
		})
	}
	type answer struct {
		Node string
		Got  map[string]json.RawMessage
	}
	// ask is the call an agent's `agent_call {function, namespace}` becomes: the id with a namespace beside it.
	ask := func(c *iii.Client, namespace string) (answer, error) {
		// 20 s: the first packets between two machines can wait 5 s for a WireGuard retry.
		out, err := c.Call(f.ctx, iii.CallRequest{FunctionID: "shell::exec", Namespace: namespace, Payload: json.RawMessage(stamped), Timeout: 20 * time.Second})
		var a answer
		if err == nil {
			err = json.Unmarshal(out, &a)
		}
		delete(a.Got, "_caller_worker_id") // every engine stamps its own on a call it delivers
		return a, err
	}
	reaches := func(what string, c *iii.Client, namespace, node string) {
		t.Helper()
		eventually(t, 45*time.Second, what, func() error {
			a, err := ask(c, namespace)
			if err != nil {
				return err
			}
			if a.Node != node || len(a.Got) != 1 || string(a.Got["command"]) != `"hostname"` {
				return fmt.Errorf("%s answered with %v", a.Node, a.Got)
			}
			return nil
		})
	}
	// The hub's agent: the hub engine holds each machine in a namespace named after it.
	reaches("the hub calls alpha", f.admin, "alpha", "alpha")
	reaches("the hub calls beta", f.admin, "beta", "beta")
	// A machine's agent: its engine mirrors the other machine the same way.
	onAlpha := connect(t, f.ctx, alpha.engineURL, "agent", alpha.local)
	onBeta := connect(t, f.ctx, beta.engineURL, "agent", beta.local)
	reaches("alpha calls beta", onAlpha, "beta", "beta")
	reaches("beta calls alpha", onBeta, "alpha", "alpha")
	// No namespace is the caller's own project: its own shell, with nothing dropped on the way.
	if a, err := ask(onAlpha, ""); err != nil || a.Node != "alpha" || len(a.Got) != 4 {
		t.Fatalf("alpha's own shell: %+v %v", a, err)
	}
	// One engine lists both copies apart.
	out := mustCall(t, onAlpha, "default", "engine::functions::list", map[string]any{})
	var list struct{ Functions []meshclient.Row }
	if err := json.Unmarshal(out, &list); err != nil {
		t.Fatal(err)
	}
	where := []string{}
	for _, r := range list.Functions {
		if r.FunctionID == "shell::exec" {
			where = append(where, r.Namespace)
		}
	}
	if slices.Sort(where); !slices.Equal(where, []string{"alpha-local", "beta"}) {
		t.Fatalf("alpha's engine lists shell::exec in %v", where)
	}
}

// A machine still on the prefix model declares the hub namespace, as its bridge did.
func TestE2EAnOldMachineIsShutOut(t *testing.T) {
	f := newFixture(t, "project")
	n := f.enroll("old")
	c := iii.New(n.url, iii.WithOtel(iii.OtelConfig{Enabled: new(bool)}), iii.WithName("mesh-bridge"), iii.WithNamespace(f.ns))
	t.Cleanup(func() { c.Close() })
	if _, err := iii.RegisterFunctionTyped(c, "shell::exec", func(context.Context, json.RawMessage) (bool, error) { return true, nil }); err != nil {
		t.Fatal(err)
	}
	go c.Connect(f.ctx)
	eventually(t, 15*time.Second, "the proxy rejects the old machine", func() error {
		if c.FatalError() == nil {
			return errors.New("still accepted")
		}
		return nil
	})
	// Its registrations leave with its connection, which the proxy drops once the refusal is flushed.
	eventually(t, 10*time.Second, "nothing of the old machine stays registered", func() error {
		out, err := call(f.ctx, f.admin, "default", "engine::functions::list", map[string]any{"include_internal": true})
		var list struct{ Functions []meshclient.Row }
		if err == nil {
			err = json.Unmarshal(out, &list)
		}
		for _, r := range list.Functions {
			if err == nil && r.FunctionID == "shell::exec" {
				err = fmt.Errorf("%s is still in namespace %s", r.FunctionID, r.Namespace)
			}
		}
		return err
	})
}
