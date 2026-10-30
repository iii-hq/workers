package hub

import (
	"context"
	"encoding/json"
	"errors"
	"reflect"
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

type forwardingRPC struct {
	calls []iii.TriggerRequest
	err   error
}

func (f *forwardingRPC) Trigger(_ context.Context, r iii.TriggerRequest) (json.RawMessage, error) {
	f.calls = append(f.calls, r)
	return json.RawMessage(`{"target":"result"}`), f.err
}
func TestMiddlewareRefusesWithoutSessionKey(t *testing.T) {
	h := authHub()
	f := &forwardingRPC{}
	for _, ctx := range []SessionContext{{}, {MeshNode: "laptop", SessionKey: "invalid"}, {MeshNode: "desktop", SessionKey: h.Sessions.Open("laptop")}} {
		if _, err := h.middleware(t.Context(), f, MiddlewareRequest{FunctionID: "shell::exec", Context: ctx}); err == nil {
			t.Fatal("untrusted context forwarded")
		}
	}
	if len(f.calls) != 0 {
		t.Fatal("refused request reached engine")
	}
}
func TestMiddlewareConfigurationRules(t *testing.T) {
	h := authHub()
	ctx := SessionContext{MeshNode: "laptop", SessionKey: h.Sessions.Open("laptop")}
	for _, tc := range []struct {
		name, id, payload string
		allow             bool
	}{
		{"raw get", "configuration::get", `{"id":"laptop__ide","raw":false}`, true},
		{"foreign get", "configuration::get", `{"id":"mesh"}`, false},
		{"prefix collision", "configuration::get", `{"id":"laptopx__ide"}`, false},
		{"invalid id", "configuration::get", `{"id":"laptop__../mesh"}`, false},
		{"empty suffix", "configuration::get", `{"id":"laptop__"}`, false},
		{"template value", "configuration::ensure", `{"id":"laptop__ide","value":"${OPENAI_API_KEY}"}`, false},
		{"nested template", "configuration::ensure", `{"id":"laptop__ide","initial_value":{"a":{"b":"x${X}"}}}`, false},
		{"array template", "configuration::set", `{"id":"laptop__ide","value":["${X}"]}`, false},
		{"register template", "configuration::register", `{"id":"laptop__ide","initial_value":"${X}"}`, false},
		{"safe ensure", "configuration::ensure", `{"id":"laptop__ide","initial_value":{"a":false}}`, true},
		{"delete", "configuration::delete", `{"id":"laptop__ide"}`, false},
		{"shell", "shell::exec", `{"command":"pwd"}`, true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			f := &forwardingRPC{}
			request := MiddlewareRequest{FunctionID: tc.id, Payload: json.RawMessage(tc.payload), Namespace: "default", Context: ctx}
			got, err := h.middleware(t.Context(), f, request)
			if (err == nil) != tc.allow {
				t.Fatalf("allow=%v err=%v", tc.allow, err)
			}
			if !tc.allow {
				if len(f.calls) != 0 {
					t.Fatal("denied call forwarded")
				}
				return
			}
			if string(got) != `{"target":"result"}` || len(f.calls) != 1 {
				t.Fatalf("forward: %s %+v", got, f.calls)
			}
			call := f.calls[0]
			if call.FunctionID != tc.id || call.Namespace != "default" || call.Timeout != 610*time.Second {
				t.Fatalf("target: %+v", call)
			}
			if tc.id == "configuration::get" {
				var p map[string]any
				json.Unmarshal(call.Data, &p)
				if p["raw"] != true {
					t.Fatal("expanded get")
				}
			} else if string(call.Data) != tc.payload {
				t.Fatal("changed ordinary payload")
			}
		})
	}
	f := &forwardingRPC{err: errors.New("target failed")}
	action := iii.VoidAction()
	_, err := h.middleware(t.Context(), f, MiddlewareRequest{FunctionID: "shell::exec", Namespace: "proj", Context: ctx, Action: action, Payload: json.RawMessage(`null`)})
	if err != f.err || !reflect.DeepEqual(f.calls[0].Action, action) || f.calls[0].Namespace != "proj" {
		t.Fatal("target action/error/namespace lost")
	}
}
