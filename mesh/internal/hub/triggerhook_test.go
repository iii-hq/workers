package hub

import (
	"encoding/json"
	"testing"
)

func TestTriggerHookRules(t *testing.T) {
	h := authHub()
	ctx := SessionContext{MeshNode: "laptop", SessionKey: h.Sessions.Open("laptop")}
	for _, tc := range []struct {
		name, kind, config string
		allow              bool
	}{
		{"hook", "harness::hook::pre-trigger", `{}`, false},
		{"foreign config", "configuration", `{"configuration_id":"mesh"}`, false},
		{"own config", "configuration", `{"configuration_id":"laptop__ide","event_types":["configuration:updated"]}`, true},
		{"global filter", "configuration", `{}`, false},
		{"wrong alias", "configuration", `{"id":"laptop__ide"}`, false},
		{"condition", "configuration", `{"configuration_id":"laptop__ide","condition_function_id":"shell::exec"}`, false},
		{"null condition", "configuration", `{"configuration_id":"laptop__ide","condition_function_id":null}`, false},
		{"unknown key", "configuration", `{"configuration_id":"laptop__ide","surprise":true}`, false},
		{"wrong events", "configuration", `{"configuration_id":"laptop__ide","event_types":"configuration:updated"}`, false},
		{"foreign type", "desktop::event", `{}`, false},
		{"own type", "laptop::event", `{}`, true},
		{"own null", "laptop::event", `null`, true},
		{"own unknown key", "laptop::event", `{"filter":"x"}`, false},
		{"own condition", "laptop::event", `{"condition_function_id":"shell::exec"}`, false},
		{"shared queue", "queue", `{}`, false},
		{"unknown global", "foreign", `{}`, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			req := TriggerHookRequest{TriggerID: "binding", TriggerType: tc.kind, FunctionID: "ide::changed", Config: json.RawMessage(tc.config), Context: ctx}
			got, err := h.TriggerHook(t.Context(), req)
			if (err == nil) != tc.allow {
				t.Fatalf("allow=%v err=%v", tc.allow, err)
			}
			if tc.allow && (got.TriggerID != req.TriggerID || got.TriggerType != req.TriggerType || got.FunctionID != req.FunctionID || string(got.Config) != tc.config) {
				t.Fatalf("mapping: %+v", got)
			}
		})
	}
	for _, ctx := range []SessionContext{{}, {MeshNode: "desktop", SessionKey: ctx.SessionKey}} {
		if _, err := h.TriggerHook(t.Context(), TriggerHookRequest{TriggerType: "configuration", Config: json.RawMessage(`{"configuration_id":"laptop__ide"}`), Context: ctx}); err == nil {
			t.Fatal("untrusted context accepted")
		}
	}
}

func TestTriggerHookResolvedIdentity(t *testing.T) {
	h := authHub()
	ctx := SessionContext{SessionKey: h.Sessions.Open("laptop"), MeshNode: "laptop"}
	for _, tc := range []struct {
		raw, resolved string
		allow         bool
	}{
		{"echo", "laptop::echo", true},
		{"echo", "echo", false},
		{"echo", "desktop::echo", false},
		{"configuration", "laptop::configuration", false},
		{"harness::hook::pre-trigger", "laptop::harness::hook::pre-trigger", false},
		{"queue", "laptop::queue", false},
	} {
		r := TriggerHookRequest{TriggerID: "binding", TriggerType: tc.raw, ResolvedTriggerType: tc.resolved, FunctionID: "echo::received", Config: json.RawMessage(`{}`), Context: ctx}
		got, err := h.TriggerHook(t.Context(), r)
		if (err == nil) != tc.allow {
			t.Fatalf("%s => %s: %v", tc.raw, tc.resolved, err)
		}
		if tc.allow && got.TriggerType != tc.raw {
			t.Fatal("raw type remapped twice")
		}
	}
}
