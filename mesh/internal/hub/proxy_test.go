package hub

import (
	"encoding/json"
	"reflect"
	"testing"
)

func TestEnsureProxyConfig(t *testing.T) {
	next, write, status := EnsureProxyConfig(nil)
	want := map[string]any{"host": "127.0.0.1", "port": 49200, "engine_url": "ws://127.0.0.1:49134", "middleware_function_id": "mesh::middleware", "rbac": map[string]any{"auth_function_id": "mesh::auth", "on_observer_registration_function_id": "mesh::on-observer-registration"}}
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

func TestMigrateOnlyOwnedLegacyProxyHook(t *testing.T) {
	legacy := map[string]any{"host": "127.0.0.1", "port": float64(50200), "engine_url": "ws://127.0.0.1:50201", "middleware_function_id": "mesh::middleware", "operator_limit": float64(7), "rbac": map[string]any{"auth_function_id": "mesh::auth", "on_trigger_registration_function_id": "mesh::on-trigger-registration", "operator_label": "keep"}}
	before, _ := json.Marshal(legacy)
	next, write, status := EnsureProxyConfig(legacy)
	if !write || status != "ok" {
		t.Fatalf("legacy Mesh hook was not migrated: write=%v status=%s", write, status)
	}
	rbac := next["rbac"].(map[string]any)
	if rbac["on_observer_registration_function_id"] != "mesh::on-observer-registration" {
		t.Fatal("missing current hook")
	}
	if _, ok := rbac["on_trigger_registration_function_id"]; ok {
		t.Fatal("removed hook retained")
	}
	if next["port"] != float64(50200) || next["engine_url"] != "ws://127.0.0.1:50201" || next["operator_limit"] != float64(7) || rbac["operator_label"] != "keep" {
		t.Fatalf("operator configuration changed: %+v", next)
	}
	after, _ := json.Marshal(legacy)
	if string(before) != string(after) {
		t.Fatal("migration mutated authoritative input before persistence")
	}
	for _, mutate := range []func(map[string]any){
		func(c map[string]any) { c["middleware_function_id"] = "other::middleware" },
		func(c map[string]any) { c["rbac"].(map[string]any)["auth_function_id"] = "other::auth" },
		func(c map[string]any) {
			c["rbac"].(map[string]any)["on_trigger_registration_function_id"] = "other::hook"
		},
		func(c map[string]any) {
			c["rbac"].(map[string]any)["on_observer_registration_function_id"] = "other::hook"
		},
		func(c map[string]any) {
			c["rbac"].(map[string]any)["on_function_registration_function_id"] = "other::hook"
		},
		func(c map[string]any) { c["rbac"].(map[string]any)["expose_functions"] = []any{"state::*"} },
	} {
		var current map[string]any
		json.Unmarshal(before, &current)
		mutate(current)
		beforeConflict, _ := json.Marshal(current)
		result, write, _ := EnsureProxyConfig(current)
		afterConflict, _ := json.Marshal(result)
		if write || string(beforeConflict) != string(afterConflict) {
			t.Fatal("conflicting ownership/configuration was changed")
		}
		if validProxyHooks(result) {
			t.Fatal("conflicting legacy configuration accepted")
		}
	}
}
