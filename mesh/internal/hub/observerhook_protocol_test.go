package hub

import (
	"encoding/json"
	"testing"
)

// The hook receives the real proxy payload, including resolved IDs derived from
// registrations. Bare names alone are not an ownership claim.
func TestObserverHookProxyPayload(t *testing.T) {
	h := authHub()
	session := SessionContext{MeshNode: "laptop", SessionKey: h.Sessions.Open("laptop")}
	for _, tc := range []struct {
		name, source, resolved, ns, args, filters string
		allow                                     bool
	}{
		{"own bare", "echo::event", "laptop::echo::event", "proj", `{}`, `{}`, true},
		{"hub collision", "echo::event", "echo::event", "proj", `{}`, `{}`, false},
		{"foreign node", "echo::event", "desktop::echo::event", "proj", `{}`, `{}`, false},
		{"foreign namespace", "echo::event", "laptop::echo::event", "foreign", `{}`, `{}`, false},
		{"own config", "configuration::on-updated", "configuration::on-updated", "default", `{}`, `{"input.id":{"$eq":"laptop__ide"},"input.event_type":{"$eq":"configuration:updated"}}`, true},
		{"all config", "configuration::on-updated", "configuration::on-updated", "default", `{}`, `{}`, false},
		{"foreign config", "configuration::on-updated", "configuration::on-updated", "default", `{}`, `{"input.id":{"$eq":"desktop__ide"}}`, false},
		{"broad config", "configuration::on-updated", "configuration::on-updated", "default", `{}`, `{"input.id":{"$in":["laptop__ide","mesh"]}}`, false},
		{"injected args", "configuration::on-updated", "configuration::on-updated", "default", `{"condition_function_id":"evil"}`, `{"input.id":{"$eq":"laptop__ide"}}`, false},
		{"condition filter", "configuration::on-updated", "configuration::on-updated", "default", `{}`, `{"input.id":{"$eq":"laptop__ide"},"condition_function_id":"evil"}`, false},
		{"foreign config namespace", "configuration::on-updated", "configuration::on-updated", "foreign", `{}`, `{"input.id":{"$eq":"laptop__ide"}}`, false},
		{"shared state", "state::on-changed", "state::on-changed", "proj", `{}`, `{}`, false},
		{"ade injection", "ade::ui::asset", "ade::ui::asset", "proj", `{"path":"mesh/page.js"}`, `{}`, false},
		{"missing canonical identity", "laptop::echo::event", "", "proj", `{}`, `{}`, false},
		{"own unknown args", "echo::event", "laptop::echo::event", "proj", `{"surprise":true}`, `{}`, false},
		{"own condition", "echo::event", "laptop::echo::event", "proj", `{"condition_function_id":null}`, `{}`, false},
		{"config missing id", "configuration::on-updated", "configuration::on-updated", "default", `{}`, `{"input.event_type":{"$eq":"configuration:updated"}}`, false},
		{"config malformed equality", "configuration::on-updated", "configuration::on-updated", "default", `{}`, `{"input.id":{"$eq":null}}`, false},
		{"config extra operator", "configuration::on-updated", "configuration::on-updated", "default", `{}`, `{"input.id":{"$eq":"laptop__ide","$ne":"mesh"}}`, false},
		{"config logical bypass", "configuration::on-updated", "configuration::on-updated", "default", `{}`, `{"$or":[{"input.id":{"$eq":"laptop__ide"}},{"input.id":{"$eq":"mesh"}}]}`, false},
		{"config unknown event", "configuration::on-updated", "configuration::on-updated", "default", `{}`, `{"input.id":{"$eq":"laptop__ide"},"input.event_type":{"$eq":"anything"}}`, false},
		{"config malformed events", "configuration::on-updated", "configuration::on-updated", "default", `{}`, `{"input.id":{"$eq":"laptop__ide"},"input.event_type":{"$in":"configuration:updated"}}`, false},
		{"config empty events", "configuration::on-updated", "configuration::on-updated", "default", `{}`, `{"input.id":{"$eq":"laptop__ide"},"input.event_type":{"$in":[]}}`, false},
		{"config registered/deleted", "configuration::on-updated", "configuration::on-updated", "default", `{}`, `{"input.id":{"$eq":"laptop__ide"},"input.event_type":{"$in":["configuration:registered","configuration:deleted"]}}`, true},
		{"harness masked as own", "harness::hook::on-pre-call", "laptop::harness::hook::on-pre-call", "proj", `{}`, `{}`, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			wire, _ := json.Marshal(map[string]any{"observer_id": "watch", "function_id": tc.source, "resolved_function_id": tc.resolved, "namespace": tc.ns, "target_function_id": "echo::received", "resolved_target_function_id": "laptop::echo::received", "target_namespace": "proj", "args": json.RawMessage(tc.args), "filters": json.RawMessage(tc.filters), "context": session})
			var req ObserverHookRequest
			if err := json.Unmarshal(wire, &req); err != nil {
				t.Fatal(err)
			}
			_, err := h.ObserverHook(t.Context(), req)
			if (err == nil) != tc.allow {
				t.Fatalf("allow=%v err=%v", tc.allow, err)
			}
		})
	}
}

func TestObserverHookRequiresLiveSessionAndOwnTarget(t *testing.T) {
	h := authHub()
	key := h.Sessions.Open("laptop")
	valid := ObserverHookRequest{FunctionID: "echo::event", ResolvedFunctionID: "laptop::echo::event", Namespace: "proj", TargetFunctionID: "echo::received", ResolvedTargetFunctionID: "laptop::echo::received", TargetNamespace: "proj", Args: json.RawMessage(`{}`), Context: SessionContext{MeshNode: "laptop", SessionKey: key}}
	for _, mutate := range []func(*ObserverHookRequest){
		func(r *ObserverHookRequest) { r.Context = SessionContext{} },
		func(r *ObserverHookRequest) { r.Context.MeshNode = "desktop" },
		func(r *ObserverHookRequest) { r.ResolvedTargetFunctionID = "desktop::echo::received" },
		func(r *ObserverHookRequest) { r.ResolvedTargetFunctionID = "echo::received" },
		func(r *ObserverHookRequest) { r.TargetNamespace = "foreign" },
		func(r *ObserverHookRequest) { r.ResolvedTargetFunctionID = "" },
	} {
		r := valid
		mutate(&r)
		if _, err := h.ObserverHook(t.Context(), r); err == nil {
			t.Fatalf("accepted untrusted target/session: %+v", r)
		}
	}
	h.Sessions.closeAll()
	if _, err := h.ObserverHook(t.Context(), valid); err == nil {
		t.Fatal("revoked session accepted")
	}
}
