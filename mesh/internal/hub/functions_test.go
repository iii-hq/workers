package hub

import (
	"context"
	"encoding/json"
	"slices"
	"testing"

	"github.com/iii-hq/workers/mesh/internal/coordinator"
)

func TestJoinCodeNeedsPublicURL(t *testing.T) {
	h := &Hub{config: DefaultConfig()}
	if _, err := h.JoinCode(context.Background(), struct{}{}); err == nil || err.Error() != "public_url is not set" {
		t.Fatalf("error = %v", err)
	}
}

func TestTakenIncludesEngineFunctionPrefixes(t *testing.T) {
	taken, err := functionPrefixes(json.RawMessage(`{"functions":[{"function_id":"shell::exec"},{"function_id":"analytics::report"}]}`))
	if err != nil {
		t.Fatal(err)
	}
	for host, want := range map[string]string{"shell": "shell-2", "analytics": "analytics-2", "fresh": "fresh"} {
		if got := coordinator.AllocateName(host, func(s string) bool { return taken[s] }); got != want {
			t.Fatalf("%s -> %s, want %s", host, got, want)
		}
	}
}

// A machine named like a shared worker that is not running yet would take that worker's ids.
func TestJoinCannotTakeASharedWorkersName(t *testing.T) {
	h, _, up := idleNodeCallHub(t)
	h.engine = &fakeEngine{}
	h.config.Share = []string{"laptop"}
	if err := h.refreshNames(t.Context()); err != nil {
		t.Fatal(err)
	}
	up()
	names := []string{}
	for _, n := range h.Coordinator.Nodes() {
		names = append(names, n.Name)
	}
	if !slices.Equal(names, []string{"laptop-2"}) {
		t.Fatalf("nodes %v", names)
	}
}

func TestFunctionPrefixesRejectMalformedListing(t *testing.T) {
	for _, raw := range []string{`{}`, `{"functions":null}`, `{"functions":[{}]}`} {
		if _, err := functionPrefixes(json.RawMessage(raw)); err == nil {
			t.Fatalf("accepted malformed listing %s", raw)
		}
	}
}

// Machines stay listed while the hub has no address, so removing one must work then.
func TestRemoveNodeNeedsNoPublicURL(t *testing.T) {
	c, err := coordinator.New(t.TempDir(), func(secret, addr string) (string, error) { return "", nil })
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	h := &Hub{config: DefaultConfig(), Coordinator: c}
	if _, err = h.RemoveNode(context.Background(), NodeRequest{Name: "ghost"}); err == nil || err.Error() != `no removable node "ghost"` {
		t.Fatalf("error = %v", err)
	}
}
