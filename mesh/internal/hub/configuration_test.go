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
	calls             []iii.TriggerRequest
	ensureErr, getErr error
	raw               json.RawMessage
	value             json.RawMessage
}

func (f *configRPC) Trigger(ctx context.Context, r iii.TriggerRequest) (json.RawMessage, error) {
	f.t.Helper()
	f.calls = append(f.calls, r)
	if r.Namespace != "default" {
		f.t.Fatal("configuration routed outside default")
	}
	var payload map[string]json.RawMessage
	if err := json.Unmarshal(r.Data, &payload); err != nil {
		f.t.Fatal(err)
	}
	if string(payload["id"]) != `"project_mesh"` {
		f.t.Fatalf("wrong config ID: %s", r.Data)
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
					json.Unmarshal(call.Data, &p)
					if _, ok := p["initial_value"]; ok != tc.seed {
						t.Fatalf("legacy seed: %s", call.Data)
					}
				}
			}
			if found != tc.register {
				t.Fatalf("legacy registration=%v", found)
			}
			var p map[string]json.RawMessage
			json.Unmarshal(f.calls[0].Data, &p)
			if string(p["metadata"]) != `{"ui_form":"mesh"}` {
				t.Fatal("missing form identity")
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
