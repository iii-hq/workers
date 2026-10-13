package main

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	validate "github.com/santhosh-tekuri/jsonschema/v6"
)

func TestCallbackWireSchemasAndResponses(t *testing.T) {
	clearCloudEnvironment(t)
	t.Setenv("III_CONFIG_NAME", "whatsapp")
	c := DefaultConfig()
	c.DataDir = t.TempDir()
	peer := newCallbackPeer(t, func(msg map[string]json.RawMessage) map[string]any {
		if string(msg["function_id"]) == `"configuration::get"` && len(msg["invocation_id"]) > 0 {
			reply := callbackInvocationReply(msg)
			reply["result"] = map[string]any{"value": c}
			return reply
		}
		return nil
	})
	manager, err := newTestManager(c)
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	defer manager.Shutdown(context.Background())
	for _, register := range []func() error{
		func() error { return registerConfiguration(ctx, peer.client, manager, false) },
		func() error { return registerCloudWebhooks(peer.client, manager, "ws://127.0.0.1:9") },
		func() error { return registerCloudFlowEndpoint(peer.client, manager, "ws://127.0.0.1:9") },
		func() error { return registerDevelopmentTunnel(ctx, peer.client, manager) },
	} {
		if err := register(); err != nil {
			t.Fatal(err)
		}
	}
	defer manager.tunnel.Close(context.Background())
	ids := []string{
		"whatsapp::configuration-id", "whatsapp::config-status", "whatsapp::on-config-change",
		"whatsapp::cloud::webhook::verify", "whatsapp::cloud::webhook::receive",
		"whatsapp::cloud::flows::endpoint", "whatsapp::cloud::tunnel::changed",
	}
	registrations := make(map[string]map[string]json.RawMessage)
	for len(registrations) < len(ids) {
		select {
		case frame := <-peer.frames:
			if string(frame["type"]) == `"registerfunction"` {
				var id string
				if err := json.Unmarshal(frame["id"], &id); err != nil {
					t.Fatal(err)
				}
				registrations[id] = frame
			}
		case <-ctx.Done():
			t.Fatalf("missing callback registrations: %v", ctx.Err())
		}
	}
	functions := make([]map[string]any, 0, len(ids))
	responses := make(map[string]*validate.Schema)
	for _, id := range ids {
		frame := registrations[id]
		functions = append(functions, map[string]any{"name": id, "request_schema": frame["request_format"], "response_schema": frame["response_format"]})
		if string(frame["metadata"]) != `{"internal":true,"trace_hidden":true}` {
			t.Errorf("%s lost internal secret-safe metadata", id)
		}
		for _, field := range []string{"request_format", "response_format"} {
			var schema map[string]any
			if json.Unmarshal(frame[field], &schema) != nil || len(schema) == 0 {
				t.Errorf("%s has no %s on the actual registration frame", id, field)
				continue
			}
			compiler := validate.NewCompiler()
			uri := "https://whatsapp.invalid/callback.json"
			if err := compiler.AddResource(uri, schema); err != nil {
				t.Fatal(err)
			}
			compiled, err := compiler.Compile(uri)
			if err != nil {
				t.Fatalf("%s %s: %v", id, field, err)
			}
			if field == "response_format" {
				responses[id] = compiled
			} else {
				fixture := map[string]any{"_caller_worker_id": "transport", "future_field": map[string]any{"any": []any{true, 1, "value"}}}
				if strings.Contains(id, "webhook::verify") {
					fixture["query_params"] = map[string]any{"hub.mode": "subscribe", "future_parameter": "extensible"}
				}
				if strings.HasSuffix(id, "webhook::receive") || strings.HasSuffix(id, "flows::endpoint") {
					fixture["headers"] = map[string]any{"x-future-header": "value"}
					fixture["request_body"] = map[string]any{"channel_id": "channel", "access_key": "key", "direction": "read"}
				}
				if err := compiled.Validate(fixture); err != nil {
					t.Errorf("%s rejects extensible transport payload: %v", id, err)
				}
			}
		}
	}
	fixture, err := json.Marshal(map[string]any{"functions": functions, "triggers": []any{}})
	if err != nil {
		t.Fatal(err)
	}
	interfacePath := filepath.Join(t.TempDir(), "callback-interface.json")
	if err := os.WriteFile(interfacePath, fixture, 0600); err != nil {
		t.Fatal(err)
	}
	gate := exec.Command("python3", "../.github/scripts/collect_worker_interface.py", "--assert-file", interfacePath, "--assert-non-empty", "--assert-typed-schemas")
	if output, err := gate.CombinedOutput(); err != nil {
		t.Errorf("callback registrations failed repository publication gate: %v\n%s", err, output)
	}
	for _, tc := range []struct{ id, input, response string }{
		{"whatsapp::configuration-id", `{}`, `{"id":"whatsapp"}`},
		{"whatsapp::on-config-change", `{"future_event":{"secret":"private-sentinel"}}`, `{"ok":true}`},
		{"whatsapp::cloud::tunnel::changed", `{"tunnel_id":"webhooks","future_field":true}`, `{"accepted":true}`},
		{"whatsapp::cloud::webhook::verify", `{"query_params":42}`, `{"status_code":400,"headers":{"content-type":"text/plain; charset=utf-8"},"body":"Invalid verification request"}`},
		{"whatsapp::cloud::webhook::receive", `{"headers":42}`, `{"status_code":400,"headers":{"content-type":"text/plain; charset=utf-8"},"body":"Invalid webhook request"}`},
		{"whatsapp::cloud::flows::endpoint", `{"headers":42}`, `{"status_code":400,"headers":{"content-type":"text/plain; charset=utf-8"},"body":"Invalid request"}`},
		{"whatsapp::cloud::webhook::verify", `{}`, `{"status_code":503,"headers":{"content-type":"text/plain; charset=utf-8"},"body":"Webhook verification is not configured"}`},
		{"whatsapp::cloud::webhook::receive", `{}`, `{"status_code":503,"headers":{"content-type":"text/plain; charset=utf-8"},"body":"Webhook delivery is not configured"}`},
		{"whatsapp::cloud::flows::endpoint", `{}`, `{"status_code":503,"headers":{"content-type":"text/plain; charset=utf-8"},"body":"Flow endpoint is not configured"}`},
	} {
		t.Run(tc.id+"/"+tc.input, func(t *testing.T) {
			raw, remote, err := peer.invoke(ctx, tc.id, json.RawMessage(tc.input))
			if err != nil || remote != nil {
				t.Fatalf("callback replaced its response with an invocation error: %v %v", err, remote)
			}
			if string(raw) != tc.response {
				t.Errorf("callback wire response changed: got %s, want %s", raw, tc.response)
			}
			validateCallbackResponse(t, responses[tc.id], raw)
		})
	}
	raw, remote, err := peer.invoke(ctx, "whatsapp::config-status", json.RawMessage(`{}`))
	if err != nil || remote != nil {
		t.Fatalf("config status: %v %v", err, remote)
	}
	validateCallbackResponse(t, responses["whatsapp::config-status"], raw)
	if strings.Contains(string(raw), "private-sentinel") {
		t.Fatal("callback configuration status exposed input secrets")
	}
}

func validateCallbackResponse(t *testing.T, schema *validate.Schema, raw json.RawMessage) {
	t.Helper()
	if schema == nil {
		return // Missing schemas are reported at registration above.
	}
	var value any
	if err := json.Unmarshal(raw, &value); err != nil {
		t.Fatal(err)
	}
	if err := schema.Validate(value); err != nil {
		t.Errorf("callback response does not match published schema: %v", err)
	}
}
