package hub

import (
	"context"
	"encoding/json"
	"reflect"
	"testing"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

type configRPC struct {
	t                 *testing.T
	calls             []iii.CallRequest
	ensureErr, getErr error
	raw               json.RawMessage
	value             json.RawMessage
}

func (f *configRPC) Call(ctx context.Context, r iii.CallRequest) (json.RawMessage, error) {
	f.t.Helper()
	f.calls = append(f.calls, r)
	if r.Namespace != "default" {
		f.t.Fatal("configuration routed outside default")
	}
	var payload map[string]json.RawMessage
	if err := json.Unmarshal(r.Payload, &payload); err != nil {
		f.t.Fatal(err)
	}
	if string(payload["id"]) != `"project_mesh"` {
		f.t.Fatalf("wrong config ID: %s", r.Payload)
	}
	switch r.FunctionID {
	case "configuration::ensure":
		return nil, f.ensureErr
	case "configuration::register":
		return nil, nil
	case "configuration::get":
		if string(payload["raw"]) == "true" {
			return f.raw, f.getErr
		}
		return f.value, nil
	default:
		f.t.Fatalf("unexpected RPC: %+v", r)
		return nil, nil
	}
}
func TestConfigurationInitialization(t *testing.T) {
	for _, tc := range []struct {
		name, code, raw, getCode string
		register, seed, wantErr  bool
	}{
		{name: "atomic"},
		{name: "legacy preserve false", code: "function_not_found", raw: `{"value":false}`, register: true},
		{name: "legacy preserve templates", code: "function_not_found", raw: `{"value":{"public_url":"${MESH_URL}"}}`, register: true},
		{name: "legacy missing", code: "function_not_found", getCode: "NOT_FOUND", register: true, seed: true},
		{name: "legacy null", code: "function_not_found", raw: `{"value":null}`, register: true, seed: true},
		{name: "malformed", code: "function_not_found", raw: `{}`, wantErr: true},
		{name: "adapter", code: "ADAPTER_ERROR", wantErr: true},
		{name: "get unavailable", code: "function_not_found", getCode: "function_not_found", wantErr: true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			f := &configRPC{t: t, raw: json.RawMessage(tc.raw), value: json.RawMessage(`{"value":{"gateway_port":50123}}`)}
			if tc.code != "" {
				f.ensureErr = &iii.InvocationError{Code: tc.code}
			}
			if tc.getCode != "" {
				f.getErr = &iii.InvocationError{Code: tc.getCode}
			}
			cfg, err := ensureConfig(t.Context(), f, "project_mesh")
			if (err != nil) != tc.wantErr {
				t.Fatalf("error=%v", err)
			}
			if !tc.wantErr && cfg.GatewayPort != 50123 {
				t.Fatalf("stored config lost: %+v", cfg)
			}
			found := false
			for _, call := range f.calls {
				if call.FunctionID == "configuration::register" {
					found = true
					var p map[string]json.RawMessage
					json.Unmarshal(call.Payload, &p)
					if _, ok := p["initial_value"]; ok != tc.seed {
						t.Fatalf("legacy seed: %s", call.Payload)
					}
				}
			}
			if found != tc.register {
				t.Fatalf("legacy registration=%v", found)
			}
			var p map[string]json.RawMessage
			json.Unmarshal(f.calls[0].Payload, &p)
			if string(p["metadata"]) != `{"ui_form":"mesh"}` {
				t.Fatal("missing form identity")
			}
			var schema map[string]json.RawMessage
			json.Unmarshal(p["schema"], &schema)
			if _, ok := schema["required"]; ok {
				t.Fatalf("a bare compose override such as {mode: client} would fail validation: %s", p["schema"])
			}
			var properties map[string]json.RawMessage
			json.Unmarshal(schema["properties"], &properties)
			for _, key := range []string{"links", "nodes", "drives"} {
				if string(properties[key]) != `{"description":"Retired on 2026-11-10; ignored."}` {
					t.Fatalf("the engine would refuse a stored %s on configuration::get: %s", key, properties[key])
				}
			}
			if string(schema["additionalProperties"]) != "false" {
				t.Fatalf("schema accepts unknown keys: %s", p["schema"])
			}
		})
	}
}
func TestConfigChangeRefetchesAndRetainsLastGood(t *testing.T) {
	h := &Hub{ConfigID: "project_mesh", config: DefaultConfig()}
	// An unrelated event must not fetch or apply untrusted payload data.
	f := &configRPC{t: t, value: json.RawMessage(`{"value":{"gateway_port":0}}`)}
	if _, err := h.onConfigChange(t.Context(), f, ConfigChangeEvent{ID: "another"}); err != nil {
		t.Fatal(err)
	}
	if len(f.calls) != 0 {
		t.Fatal("wrong entry fetched")
	}
	before := h.Config()
	if _, err := h.onConfigChange(t.Context(), f, ConfigChangeEvent{ID: "project_mesh"}); err == nil {
		t.Fatal("invalid authoritative config accepted")
	}
	if !reflect.DeepEqual(before, h.Config()) {
		t.Fatal("invalid reload replaced last good")
	}
}
func TestConfigFetchFailure(t *testing.T) {
	f := &configRPC{t: t, value: json.RawMessage(`{}`)}
	if _, err := fetchConfig(t.Context(), f, "project_mesh"); err == nil {
		t.Fatal("malformed fetch accepted")
	}
}

func TestResumedReadsEverythingAgain(t *testing.T) {
	h, _ := tunnelHub(t, true)
	h.ConfigID, h.proxyKick = "project_mesh", make(chan struct{}, 1)
	// The stored entry changed while no event could reach the hub.
	h.engine = &configRPC{t: t, value: json.RawMessage(`{"value":{"trusted_proxies":["192.0.2.0/24"]}}`)}
	h.resumed(t.Context())
	if got := h.Config().TrustedProxies; !reflect.DeepEqual(got, []string{"192.0.2.0/24"}) {
		t.Fatalf("trusted_proxies %v: the stored configuration was not read again", got)
	}
	if nodes, proxy, tunnel := h.refreshing.Load(), len(h.proxyKick), len(h.tunnelKick); !nodes || proxy != 1 || tunnel != 1 {
		t.Fatalf("passes asked for: nodes %v, proxy %d, tunnel %d; want all three", nodes, proxy, tunnel)
	}
}
