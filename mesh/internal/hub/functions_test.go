package hub

import (
	"context"
	"encoding/json"
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

func TestFunctionPrefixesRejectMalformedListing(t *testing.T) {
	for _, raw := range []string{`{}`, `{"functions":null}`, `{"functions":[{}]}`} {
		if _, err := functionPrefixes(json.RawMessage(raw)); err == nil {
			t.Fatalf("accepted malformed listing %s", raw)
		}
	}
}
