package hub

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"reflect"
	"strings"
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

type forwardingRPC struct {
	calls []iii.CallRequest
	err   error
}

func (f *forwardingRPC) Call(_ context.Context, r iii.CallRequest) (json.RawMessage, error) {
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

// A node that says it changed is answered here, for its own session only, and reaches no engine function.
func TestNodeChangedIsAnsweredByTheMiddleware(t *testing.T) {
	h := authHub()
	f := &forwardingRPC{}
	laptop := SessionContext{MeshNode: "laptop", SessionKey: h.Sessions.Open("laptop")}
	if out, err := h.middleware(t.Context(), f, MiddlewareRequest{FunctionID: "mesh::node::changed", Namespace: "proj", Context: laptop, Action: iii.VoidAction()}); err != nil || string(out) != "null" {
		t.Fatalf("answer %s %v", out, err)
	}
	for _, ctx := range []SessionContext{{}, {MeshNode: "laptop", SessionKey: "invalid"}, {MeshNode: "desktop", SessionKey: laptop.SessionKey}} {
		if _, err := h.middleware(t.Context(), f, MiddlewareRequest{FunctionID: "mesh::node::changed", Namespace: "proj", Context: ctx}); err == nil {
			t.Fatalf("answered for %+v", ctx)
		}
	}
	if len(f.calls) != 0 {
		t.Fatalf("reached the engine: %+v", f.calls)
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
				json.Unmarshal(call.Payload, &p)
				if p["raw"] != true {
					t.Fatal("expanded get")
				}
			} else if string(call.Payload) != tc.payload {
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

// meshHub has desk and laptop on its version, the hub sharing shell and laptop sharing coder.
func meshHub() (*Hub, SessionContext) {
	h := authHub()
	h.Version = "0.2.0"
	h.config.Share = []string{"shell"}
	h.shared = shared{rows: map[string]sharedRow{"shell::exec": {"hub", "shell", false}, "laptop::coder::read-file": {"laptop", bridgeWorker("laptop"), false}}}
	h.nodeDetails = map[string]NodeDetails{"desk": {NodeEntry: NodeEntry{Version: "0.2.0"}}, "laptop": {NodeEntry: NodeEntry{Version: "0.2.0"}}}
	return h, SessionContext{MeshNode: "desk", SessionKey: h.Sessions.Open("desk")}
}
func meshRequest(ctx SessionContext, owner, id, payload string) MiddlewareRequest {
	return MiddlewareRequest{FunctionID: "mesh::call", Namespace: "proj", Context: ctx,
		Payload: json.RawMessage(`{"owner":"` + owner + `","function_id":"` + id + `","payload":` + payload + `}`)}
}
func code(err error) string {
	var ie *iii.InvocationError
	if errors.As(err, &ie) {
		return ie.Code
	}
	return fmt.Sprint(err)
}

func TestMeshCallForwardsSharedFunctions(t *testing.T) {
	h, desk := meshHub()
	for _, c := range []struct{ owner, id string }{{"hub", "shell::exec"}, {"laptop", "laptop::coder::read-file"}} {
		f := &forwardingRPC{}
		got, err := h.middleware(t.Context(), f, meshRequest(desk, c.owner, c.id, `{"a":1}`))
		if err != nil || string(got) != `{"target":"result"}` || len(f.calls) != 1 {
			t.Fatalf("%s: %s %v %+v", c.id, got, err, f.calls)
		}
		if call := f.calls[0]; call.FunctionID != c.id || call.Namespace != "proj" || string(call.Payload) != `{"a":1}` || call.Action != nil || call.Timeout != 610*time.Second {
			t.Fatalf("forwarded %+v", call)
		}
	}
	if h.inFlight["desk"] != 0 {
		t.Fatalf("calls in flight %d", h.inFlight["desk"])
	}
}

func TestMeshCallRefusals(t *testing.T) {
	ref := `{"reader":{"channel_id":"c","access_key":"k","direction":"read"}}`
	for _, c := range []struct {
		name, code string
		change     func(*Hub, *MiddlewareRequest)
		owner, id  string
		payload    string
	}{
		{"another namespace", "FORBIDDEN", func(_ *Hub, r *MiddlewareRequest) { r.Namespace = "default" }, "hub", "shell::exec", `{}`},
		{"an enqueued call", "FORBIDDEN", func(_ *Hub, r *MiddlewareRequest) { r.Action = iii.EnqueueAction("q") }, "hub", "shell::exec", `{}`},
		{"a caller on another version", "FORBIDDEN", func(h *Hub, _ *MiddlewareRequest) {
			h.nodeDetails["desk"] = NodeDetails{NodeEntry: NodeEntry{Version: "0.1.0"}}
		}, "hub", "shell::exec", `{}`},
		{"a caller never read", "mesh_offline", func(h *Hub, _ *MiddlewareRequest) { h.Version = ""; delete(h.nodeDetails, "desk") }, "hub", "shell::exec", `{}`},
		{"before the first catalog read", "mesh_offline", func(h *Hub, _ *MiddlewareRequest) { h.shared.rows = nil }, "hub", "shell::exec", `{}`},
		{"an unshared hub function", "FORBIDDEN", nil, "hub", "coder::read-file", `{}`},
		{"the wrong owner", "FORBIDDEN", nil, "hub", "laptop::coder::read-file", `{}`},
		{"a stale catalog after an unshare", "FORBIDDEN", func(h *Hub, _ *MiddlewareRequest) { h.config.Share = nil }, "hub", "shell::exec", `{}`},
		{"a node function", "FORBIDDEN", nil, "laptop", "laptop::mesh-node::workers::apply", `{}`},
		{"a mesh function", "FORBIDDEN", nil, "hub", "mesh::join::code", `{}`},
		{"an engine function", "FORBIDDEN", nil, "hub", "engine::functions::list", `{}`},
		{"no owner", "FORBIDDEN", nil, "", "engine::functions::list", `{}`},
		{"the 65th call in flight", "mesh_busy", func(h *Hub, _ *MiddlewareRequest) { h.inFlight = map[string]int{"desk": maxCallsInFlight} }, "hub", "shell::exec", `{}`},
		{"a channel ref sent", "channels_unsupported", nil, "hub", "shell::exec", ref},
	} {
		t.Run(c.name, func(t *testing.T) {
			h, desk := meshHub()
			req := meshRequest(desk, c.owner, c.id, c.payload)
			if c.change != nil {
				c.change(h, &req)
			}
			f := &forwardingRPC{}
			if _, err := h.middleware(t.Context(), f, req); code(err) != c.code || len(f.calls) != 0 {
				t.Fatalf("got %v, %d calls; want %s", err, len(f.calls), c.code)
			}
		})
	}
	// Its own function: laptop calls what it shares.
	h, _ := meshHub()
	laptop := SessionContext{MeshNode: "laptop", SessionKey: h.Sessions.Open("laptop")}
	f := &forwardingRPC{}
	if _, err := h.middleware(t.Context(), f, meshRequest(laptop, "laptop", "laptop::coder::read-file", `{}`)); code(err) != "FORBIDDEN" || len(f.calls) != 0 {
		t.Fatalf("own function: %v", err)
	}
	// A channel ref in the result is refused after the call, which leaves the count.
	h, desk := meshHub()
	result := &resultRPC{out: json.RawMessage(`{"data":` + ref + `}`)}
	if _, err := h.middleware(t.Context(), result, meshRequest(desk, "hub", "shell::exec", `{}`)); code(err) != "channels_unsupported" || h.inFlight["desk"] != 0 {
		t.Fatalf("result with a channel ref: %v, %d in flight", err, h.inFlight["desk"])
	}
}

// A function cited by id passes, internal or not; an internal one only its worker's entry covers never does.
func TestMeshCallGatesFunctionsByID(t *testing.T) {
	h, desk := meshHub()
	h.config.Share = []string{"shell", "harness::send"}
	h.shared.rows["harness::send"] = sharedRow{"hub", "harness", true}
	h.shared.rows["shell::spawn"] = sharedRow{"hub", "shell", true} // as a read racing a re-registration could leave it
	f := &forwardingRPC{}
	if _, err := h.middleware(t.Context(), f, meshRequest(desk, "hub", "harness::send", `{}`)); err != nil || len(f.calls) != 1 || f.calls[0].FunctionID != "harness::send" {
		t.Fatalf("cited internal function: %v %+v", err, f.calls)
	}
	for _, id := range []string{"shell::spawn", "shell::grant"} {
		f = &forwardingRPC{}
		if _, err := h.middleware(t.Context(), f, meshRequest(desk, "hub", id, `{}`)); code(err) != "FORBIDDEN" || len(f.calls) != 0 {
			t.Fatalf("%s: %v", id, err)
		}
	}
	// An id taken off the list leaves before the next read.
	h.config.Share = []string{"shell"}
	if _, err := h.middleware(t.Context(), f, meshRequest(desk, "hub", "harness::send", `{}`)); code(err) != "FORBIDDEN" || len(f.calls) != 0 {
		t.Fatalf("uncited id: %v", err)
	}
}

// A refusal logs what the caller sent quoted and bounded, so it cannot forge or flood log lines.
func TestMeshCallRefusalsAreLogged(t *testing.T) {
	var buf bytes.Buffer
	defer log.SetOutput(log.Writer())
	log.SetOutput(&buf)
	h, desk := meshHub()
	if _, err := h.middleware(t.Context(), &forwardingRPC{}, meshRequest(desk, "hub", strings.Repeat("a", 1000)+`\nforged`, `{}`)); code(err) != "FORBIDDEN" {
		t.Fatal(err)
	}
	ref := `{"reader":{"channel_id":"c","access_key":"k","direction":"read"}}`
	if _, err := h.middleware(t.Context(), &forwardingRPC{}, meshRequest(desk, "hub", "shell::exec", ref)); code(err) != "channels_unsupported" {
		t.Fatal(err)
	}
	lines := strings.Split(strings.TrimSuffix(buf.String(), "\n"), "\n")
	if len(lines) != 2 || len(lines[0]) > 400 || !strings.Contains(lines[1], "channel refs sent") {
		t.Fatalf("log %q", buf.String())
	}
}

type resultRPC struct{ out json.RawMessage }

func (r *resultRPC) Call(context.Context, iii.CallRequest) (json.RawMessage, error) {
	return r.out, nil
}
