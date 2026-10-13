package main

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"reflect"
	"strings"
	"sync/atomic"
	"testing"
)

func cloudTestRegistry(t *testing.T, client *cloudClient, config Config, registers ...func(*Registry, *cloudClient)) *Registry {
	t.Helper()
	config.DataDir = t.TempDir()
	manager, err := newTestManager(config)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = manager.Shutdown(context.Background()) })
	r := &Registry{Manager: manager, handlers: map[string]operationHandler{}}
	for _, register := range registers {
		register(r, client)
	}
	return r
}

func cloudActionsRegistry(t *testing.T, client *cloudClient, config Config) *Registry {
	t.Helper()
	return cloudTestRegistry(t, client, config, registerCloudActions, registerCloudInteractive)
}

func TestCloudActionsHTTP(t *testing.T) {
	var bodies []map[string]any
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		if r.Method != "POST" || r.URL.Path != "/v26.0/456/messages" || r.Header.Get("Authorization") != "Bearer test-access-secret" {
			t.Error("wrong authenticated endpoint")
		}
		var body map[string]any
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			t.Error(err)
		}
		bodies = append(bodies, body)
		if body["status"] == "read" {
			fmt.Fprint(w, `{"success":true}`)
		} else {
			fmt.Fprintf(w, `{"messages":[{"id":"wamid.%d"}]}`, len(bodies))
		}
	})
	r := cloudActionsRegistry(t, client, cloudTestConfig())
	for _, name := range []string{"mark_read", "typing"} {
		got, err := r.Invoke(context.Background(), "whatsapp::cloud::messages::"+name, map[string]any{"message_id": "wamid.original"})
		if err != nil {
			t.Fatal(err)
		}
		if got != (CloudActionResult{Success: true}) {
			t.Fatalf("bad action acknowledgement: %#v", got)
		}
	}
	wantRead := map[string]any{"messaging_product": "whatsapp", "status": "read", "message_id": "wamid.original"}
	if !reflect.DeepEqual(bodies[0], wantRead) {
		t.Fatalf("bad read payload: %#v", bodies[0])
	}
	wantRead["typing_indicator"] = map[string]any{"type": "text"}
	if !reflect.DeepEqual(bodies[1], wantRead) {
		t.Fatalf("bad typing payload: %#v", bodies[1])
	}
	for _, emoji := range []string{"👍🏽", ""} {
		got, err := r.Invoke(context.Background(), "whatsapp::cloud::messages::send_reaction", map[string]any{"chat_id": "+15551234567", "message_id": "wamid.original", "emoji": emoji})
		if err != nil {
			t.Fatal(err)
		}
		if got.(CloudSentMessage).Status != "accepted" {
			t.Fatal("acceptance must not claim delivery")
		}
		if !reflect.DeepEqual(bodies[len(bodies)-1]["reaction"], map[string]any{"message_id": "wamid.original", "emoji": emoji}) {
			t.Fatal("reaction payload differs")
		}
	}
	_, err := r.Invoke(context.Background(), "whatsapp::cloud::messages::send_location", map[string]any{"chat_id": "+15551234567", "latitude": 0.0, "longitude": 0.0, "name": "Origin", "reply_to_message_id": "wamid.original"})
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(bodies[4]["location"], map[string]any{"latitude": 0.0, "longitude": 0.0, "name": "Origin"}) || !reflect.DeepEqual(bodies[4]["context"], map[string]any{"message_id": "wamid.original"}) {
		t.Fatalf("bad location: %#v", bodies[4])
	}
	_, err = r.Invoke(context.Background(), "whatsapp::cloud::messages::send_contacts", map[string]any{"chat_id": "+15551234567", "contacts": []any{map[string]any{"name": map[string]any{"formatted_name": "Alex", "first_name": "Alex"}, "phones": []any{map[string]any{"phone": "+15557654321", "wa_id": "15557654321", "type": "WORK"}}}}})
	if err != nil {
		t.Fatal(err)
	}
	if bodies[5]["type"] != "contacts" {
		t.Fatal("wrong contacts type")
	}
}

func TestCloudActionsInvalidBeforeHTTP(t *testing.T) {
	var calls atomic.Int32
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) { calls.Add(1) })
	registry := cloudActionsRegistry(t, client, cloudTestConfig())
	tests := []struct {
		name  string
		input map[string]any
	}{
		{"mark_read", map[string]any{"message_id": ""}},
		{"typing", map[string]any{"message_id": "bad\n"}},
		{"send_reaction", map[string]any{"chat_id": "+15551234567", "message_id": "wamid.1", "emoji": "not emoji"}},
		{"send_reaction", map[string]any{"chat_id": "+15551234567", "message_id": "wamid.1"}},
		{"send_location", map[string]any{"chat_id": "+15551234567", "latitude": 91, "longitude": 0}},
		{"send_location", map[string]any{"chat_id": "+15551234567", "longitude": 0}},
		{"send_contacts", map[string]any{"chat_id": "+15551234567", "contacts": []any{}}},
		{"send_contacts", map[string]any{"chat_id": "+15551234567", "contacts": []any{map[string]any{"name": map[string]any{"formatted_name": "Alex"}, "birthday": "2026-02-31"}}}},
		{"send_contacts", map[string]any{"chat_id": "+15551234567", "contacts": []any{map[string]any{"name": map[string]any{"formatted_name": "Alex"}, "host_path": "/tmp/secret"}}}},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			_, err := registry.Invoke(context.Background(), "whatsapp::cloud::messages::"+tc.name, tc.input)
			assertCloudDocumentError(t, err, "invalid_request")
		})
	}
	if calls.Load() != 0 {
		t.Fatal("invalid request reached Meta")
	}
}

func TestCloudActionsErrorsNoRetry(t *testing.T) {
	for _, op := range []string{"mark_read", "send_reaction", "send_buttons"} {
		for _, scenario := range []string{"missing_credentials", "meta", "window", "timeout", "invalid_ack"} {
			t.Run(op+"/"+scenario, func(t *testing.T) {
				var calls atomic.Int32
				config := cloudTestConfig()
				ctx, cancel := context.WithCancel(context.Background())
				defer cancel()
				client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
					calls.Add(1)
					switch scenario {
					case "meta":
						w.WriteHeader(400)
						fmt.Fprint(w, `{"error":{"code":100,"message":"test-access-secret test-app-secret test-verify-secret"}}`)
					case "window":
						w.WriteHeader(400)
						fmt.Fprint(w, `{"error":{"code":131047}}`)
					case "timeout":
						cancel()
						w.WriteHeader(http.StatusAccepted)
					default:
						fmt.Fprint(w, `{"success":false}`)
					}
				})
				if scenario == "missing_credentials" {
					config.Cloud.AccessToken = ""
					config.Cloud.Enabled = false
				}
				registry := cloudActionsRegistry(t, client, config)
				if scenario == "missing_credentials" {
					registry.Manager.mu.Lock()
					registry.Manager.config.Cloud.Enabled = true
					registry.Manager.mu.Unlock()
				}
				input := map[string]any{"message_id": "wamid.1"}
				if op == "send_reaction" {
					input["chat_id"] = "+15551234567"
					input["emoji"] = "👍"
				}
				if op == "send_buttons" {
					input = map[string]any{"chat_id": "+15551234567", "text": "Choose", "buttons": []any{map[string]any{"id": "yes", "title": "Yes"}}}
				}
				_, err := registry.Invoke(ctx, "whatsapp::cloud::messages::"+op, input)
				code := map[string]string{"missing_credentials": "cloud_not_configured", "meta": "cloud_api_error", "window": "cloud_window_closed", "timeout": "cloud_outcome_unknown", "invalid_ack": "cloud_outcome_unknown"}[scenario]
				assertCloudDocumentError(t, err, code)
				if scenario == "timeout" && !strings.Contains(err.Error(), "No automatic retry") {
					t.Fatal("timeout ambiguity not explained")
				}
				want := int32(1)
				if scenario == "missing_credentials" {
					want = 0
				}
				if calls.Load() != want {
					t.Fatalf("requests=%d want=%d", calls.Load(), want)
				}
			})
		}
	}
}
