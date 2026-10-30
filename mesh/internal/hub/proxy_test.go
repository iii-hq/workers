package hub

import (
	"reflect"
	"testing"
)

func TestEnsureProxyConfig(t *testing.T) {
	next, write, status := EnsureProxyConfig(nil)
	want := map[string]any{"host": "127.0.0.1", "port": 49200, "engine_url": "ws://127.0.0.1:49134", "middleware_function_id": "mesh::middleware", "rbac": map[string]any{"auth_function_id": "mesh::auth", "on_trigger_registration_function_id": "mesh::on-trigger-registration"}}
	if !write || status != "ok" || !reflect.DeepEqual(next, want) {
		t.Fatalf("claim: %#v %v %s", next, write, status)
	}
	if _, write, _ := EnsureProxyConfig(next); write {
		t.Fatal("rewrote owned config")
	}
	foreign := map[string]any{"rbac": map[string]any{"auth_function_id": "other::auth"}}
	if got, write, status := EnsureProxyConfig(foreign); write || status != "owned by other::auth" || !reflect.DeepEqual(got, foreign) {
		t.Fatalf("foreign config: %#v %v %s", got, write, status)
	}
	for _, bad := range []map[string]any{{"rbac": false}, {"rbac": map[string]any{"auth_function_id": true}}} {
		if _, write, status := EnsureProxyConfig(bad); write || status == "ok" {
			t.Fatalf("claimed malformed config: %+v", bad)
		}
	}
}
