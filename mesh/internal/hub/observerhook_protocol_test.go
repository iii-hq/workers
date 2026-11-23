package hub

import (
	"encoding/json"
	"testing"
)

// The hook receives the real proxy payload. A machine's own functions live in
// its namespace, so the namespaces are the ownership claim.
func TestObserverHookProxyPayload(t *testing.T) {
	h := authHub()
	session := SessionContext{MeshNode: "laptop", SessionKey: h.Sessions.Open("laptop")}
	own := `{"input.id":{"$eq":"laptop__ide"},"input.event_type":{"$eq":"configuration:updated"}}`
	for _, tc := range []struct {
		name, source, ns, targetNS, args, filters string
		allow                                     bool
	}{
		{"own source and target", "echo::event", "laptop", "laptop", `{}`, `{}`, true},
		{"target on the hub", "echo::event", "laptop", "proj", `{}`, `{}`, false},
		{"target on another machine", "echo::event", "laptop", "desk", `{}`, `{}`, false},
		{"source on the hub", "echo::event", "proj", "laptop", `{}`, `{}`, false},
		{"source on another machine", "echo::event", "desk", "laptop", `{}`, `{}`, false},
		{"harness masked as own", "harness::hook::on-pre-call", "laptop", "laptop", `{}`, `{}`, false},
		{"shared state", "state::on-changed", "laptop", "laptop", `{}`, `{}`, false},
		{"ade injection", "ade::ui::asset", "laptop", "laptop", `{"path":"mesh/page.js"}`, `{}`, false},
		{"own unknown args", "echo::event", "laptop", "laptop", `{"surprise":true}`, `{}`, false},
		{"own condition", "echo::event", "laptop", "laptop", `{"condition_function_id":null}`, `{}`, false},
		{"own config", "configuration::on-updated", "default", "laptop", `{}`, own, true},
		{"own config, target on the hub", "configuration::on-updated", "default", "proj", `{}`, own, false},
		{"config named in the machine's namespace", "configuration::on-updated", "laptop", "laptop", `{}`, own, false},
		{"foreign config namespace", "configuration::on-updated", "foreign", "laptop", `{}`, `{"input.id":{"$eq":"laptop__ide"}}`, false},
		{"all config", "configuration::on-updated", "default", "laptop", `{}`, `{}`, false},
		{"foreign config", "configuration::on-updated", "default", "laptop", `{}`, `{"input.id":{"$eq":"desktop__ide"}}`, false},
		{"broad config", "configuration::on-updated", "default", "laptop", `{}`, `{"input.id":{"$in":["laptop__ide","mesh"]}}`, false},
		{"injected args", "configuration::on-updated", "default", "laptop", `{"condition_function_id":"evil"}`, `{"input.id":{"$eq":"laptop__ide"}}`, false},
		{"condition filter", "configuration::on-updated", "default", "laptop", `{}`, `{"input.id":{"$eq":"laptop__ide"},"condition_function_id":"evil"}`, false},
		{"config missing id", "configuration::on-updated", "default", "laptop", `{}`, `{"input.event_type":{"$eq":"configuration:updated"}}`, false},
		{"config malformed equality", "configuration::on-updated", "default", "laptop", `{}`, `{"input.id":{"$eq":null}}`, false},
		{"config extra operator", "configuration::on-updated", "default", "laptop", `{}`, `{"input.id":{"$eq":"laptop__ide","$ne":"mesh"}}`, false},
		{"config logical bypass", "configuration::on-updated", "default", "laptop", `{}`, `{"$or":[{"input.id":{"$eq":"laptop__ide"}},{"input.id":{"$eq":"mesh"}}]}`, false},
		{"config unknown event", "configuration::on-updated", "default", "laptop", `{}`, `{"input.id":{"$eq":"laptop__ide"},"input.event_type":{"$eq":"anything"}}`, false},
		{"config malformed events", "configuration::on-updated", "default", "laptop", `{}`, `{"input.id":{"$eq":"laptop__ide"},"input.event_type":{"$in":"configuration:updated"}}`, false},
		{"config empty events", "configuration::on-updated", "default", "laptop", `{}`, `{"input.id":{"$eq":"laptop__ide"},"input.event_type":{"$in":[]}}`, false},
		{"config registered/deleted", "configuration::on-updated", "default", "laptop", `{}`, `{"input.id":{"$eq":"laptop__ide"},"input.event_type":{"$in":["configuration:registered","configuration:deleted"]}}`, true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			// rbac-proxy still sends the resolved ids; the hook reads none of them.
			wire, _ := json.Marshal(map[string]any{"observer_id": "watch", "function_id": tc.source, "resolved_function_id": tc.source, "namespace": tc.ns, "target_function_id": "echo::received", "resolved_target_function_id": "echo::received", "target_namespace": tc.targetNS, "args": json.RawMessage(tc.args), "filters": json.RawMessage(tc.filters), "context": session})
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
	valid := ObserverHookRequest{FunctionID: "echo::event", Namespace: "laptop", TargetFunctionID: "echo::received", TargetNamespace: "laptop", Args: json.RawMessage(`{}`), Context: SessionContext{MeshNode: "laptop", SessionKey: key}}
	if _, err := h.ObserverHook(t.Context(), valid); err != nil {
		t.Fatal(err)
	}
	for _, mutate := range []func(*ObserverHookRequest){
		func(r *ObserverHookRequest) { r.Context = SessionContext{} },
		func(r *ObserverHookRequest) { r.Context.MeshNode = "desktop" },
		func(r *ObserverHookRequest) { r.TargetNamespace = "desktop" },
		func(r *ObserverHookRequest) { r.TargetNamespace = "proj" },
		func(r *ObserverHookRequest) { r.TargetNamespace = "" },
		func(r *ObserverHookRequest) { r.Namespace = "" },
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
