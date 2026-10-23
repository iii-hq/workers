package main

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"reflect"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func TestCloudManagementHTTPContracts(t *testing.T) {
	cases := []struct {
		operation, method, path string
		input, body             map[string]any
		query                   url.Values
		response                string
	}{
		{"templates::create", "POST", "123/message_templates", map[string]any{"name": "report_ready", "language": "en_US", "category": "UTILITY", "components": []any{map[string]any{"type": "BODY", "text": "Your report is ready."}}}, map[string]any{"name": "report_ready", "language": "en_US", "category": "UTILITY", "components": []any{map[string]any{"type": "BODY", "text": "Your report is ready."}}}, nil, `{"id":"789","status":"PENDING","category":"UTILITY"}`},
		{"templates::get", "GET", "789", map[string]any{"template_id": "789"}, nil, nil, `{"id":"789","name":"report_ready","components":[]}`},
		{"templates::update", "POST", "789", map[string]any{"template_id": "789", "category": "UTILITY"}, map[string]any{"category": "UTILITY"}, nil, `{"success":true}`},
		{"templates::delete", "DELETE", "123/message_templates", map[string]any{"name": "report_ready", "template_id": "789"}, nil, url.Values{"name": {"report_ready"}, "hsm_id": {"789"}}, `{"success":true}`},
		{"templates::unpause", "POST", "789/unpause", map[string]any{"template_id": "789"}, nil, nil, `{"success":true}`},
		{"profile::get", "GET", "456/whatsapp_business_profile", nil, nil, nil, `{"data":[{"about":"Support","websites":["https://example.com"]}]}`},
		{"profile::update", "POST", "456/whatsapp_business_profile", map[string]any{"about": "Support", "websites": []any{"https://example.com"}}, map[string]any{"messaging_product": "whatsapp", "about": "Support", "websites": []any{"https://example.com"}}, nil, `{"success":true}`},
		{"phone_numbers::get", "GET", "456", map[string]any{"fields": []any{"id", "throughput"}}, nil, url.Values{"fields": {"id,throughput"}}, `{"id":"456","throughput":{"level":"STANDARD"}}`},
		{"phone_numbers::list", "GET", "123/phone_numbers", map[string]any{"limit": 2, "after": "cursor"}, nil, nil, `{"data":[{"id":"456"}],"paging":{"cursors":{"after":"next-cursor"},"next":"https://graph.facebook.com/?access_token=test-access-secret"}}`},
		{"phone_numbers::request_code", "POST", "456/request_code", map[string]any{"code_method": "SMS", "language": "en_US"}, map[string]any{"code_method": "SMS", "language": "en_US"}, nil, `{"success":true}`},
		{"phone_numbers::verify_code", "POST", "456/verify_code", map[string]any{"code": "123456"}, map[string]any{"code": "123456"}, nil, `{"success":true}`},
		{"phone_numbers::register", "POST", "456/register", map[string]any{"pin": "123456", "data_localization_region": "BR"}, map[string]any{"messaging_product": "whatsapp", "pin": "123456", "data_localization_region": "BR"}, nil, `{"success":true}`},
		{"phone_numbers::deregister", "POST", "456/deregister", nil, nil, nil, `{"success":true}`},
		{"phone_numbers::set_pin", "POST", "456", map[string]any{"pin": "654321"}, map[string]any{"pin": "654321"}, nil, `{"success":true}`},
		{"phone_numbers::set_display_name", "POST", "456", map[string]any{"new_display_name": "Example Support"}, map[string]any{"new_display_name": "Example Support"}, nil, `{"success":true}`},
		{"subscriptions::list", "GET", "123/subscribed_apps", nil, nil, nil, `{"data":[{"whatsapp_business_api_data":{"id":"890","name":"Example"}}]}`},
		{"subscriptions::subscribe", "POST", "123/subscribed_apps", map[string]any{"override_callback_uri": "https://example.com/webhook"}, map[string]any{"override_callback_uri": "https://example.com/webhook", "verify_token": "test-verify-secret"}, nil, `{"success":true}`},
		{"subscriptions::unsubscribe", "DELETE", "123/subscribed_apps", nil, nil, nil, `{"success":true}`},
		{"commerce::get", "GET", "456/whatsapp_commerce_settings", nil, nil, nil, `{"data":[{"is_cart_enabled":true,"is_catalog_visible":false}]}`},
		{"commerce::update", "POST", "456/whatsapp_commerce_settings", map[string]any{"is_cart_enabled": false}, map[string]any{"is_cart_enabled": false}, nil, `{"success":true}`},
		{"qr_codes::create", "POST", "456/message_qrdls", map[string]any{"prefilled_message": "Help", "generate_qr_image": "SVG"}, map[string]any{"prefilled_message": "Help", "generate_qr_image": "SVG"}, nil, `{"code":"4O4YGZEG3RIVE1","prefilled_message":"Help","deep_link_url":"https://wa.me/message/4O4YGZEG3RIVE1"}`},
		{"qr_codes::list", "GET", "456/message_qrdls", map[string]any{"limit": 10}, nil, url.Values{"limit": {"10"}}, `{"data":[]}`},
		{"qr_codes::get", "GET", "456/message_qrdls", map[string]any{"code": "4O4YGZEG3RIVE1"}, nil, url.Values{"code": {"4O4YGZEG3RIVE1"}}, `{"data":[{"code":"4O4YGZEG3RIVE1"}]}`},
		{"qr_codes::update", "POST", "456/message_qrdls", map[string]any{"code": "4O4YGZEG3RIVE1", "prefilled_message": "Hello"}, map[string]any{"code": "4O4YGZEG3RIVE1", "prefilled_message": "Hello"}, nil, `{"code":"4O4YGZEG3RIVE1","prefilled_message":"Hello"}`},
		{"qr_codes::delete", "DELETE", "456/message_qrdls/4O4YGZEG3RIVE1", map[string]any{"code": "4O4YGZEG3RIVE1"}, nil, nil, `{"success":true}`},
		{"conversational_components::get", "GET", "456", nil, nil, url.Values{"fields": {"conversational_automation"}}, `{"id":"456","conversational_automation":{"prompts":["Help"]}}`},
		{"conversational_components::update", "POST", "456/conversational_automation", map[string]any{"prompts": []any{"Help"}, "commands": []any{map[string]any{"command_name": "help", "command_description": "Get help"}}}, map[string]any{"prompts": []any{"Help"}, "commands": []any{map[string]any{"command_name": "help", "command_description": "Get help"}}}, nil, `{"success":true}`},
		{"blocking::list", "GET", "456/block_users", map[string]any{"limit": 10}, nil, url.Values{"limit": {"10"}}, `{"data":[]}`},
		{"blocking::block", "POST", "456/block_users", map[string]any{"users": []any{"+15551234567"}}, map[string]any{"messaging_product": "whatsapp", "block_users": []any{map[string]any{"user": "+15551234567"}}}, nil, `{"block_users":{"added_users":[{"input":"+15551234567","wa_id":"15551234567"}]}}`},
		{"blocking::unblock", "DELETE", "456/block_users", map[string]any{"users": []any{"+15551234567"}}, map[string]any{"messaging_product": "whatsapp", "block_users": []any{map[string]any{"user": "+15551234567"}}}, nil, `{"block_users":{"removed_users":[{"input":"+15551234567"}]}}`},
		{"identity::get", "GET", "456/settings", nil, nil, nil, `{"user_identity_change":{"enable_identity_key_check":true},"calling":{"sip":{"password":"must-not-return"}}}`},
		{"identity::update", "POST", "456/settings", map[string]any{"enable_identity_key_check": true}, map[string]any{"user_identity_change": map[string]any{"enable_identity_key_check": true}}, nil, `{"success":true}`},
	}
	for _, tc := range cases {
		t.Run(tc.operation, func(t *testing.T) {
			entry, ok := catalogEntries["whatsapp::cloud::"+tc.operation]
			if !ok {
				t.Fatal("management function missing from advertised catalog")
			}
			var targetCalls atomic.Int32
			client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
				if r.Header.Get("Authorization") != "Bearer test-access-secret" || r.URL.Query().Has("access_token") {
					t.Error("incorrect credential transport")
				}
				if r.Method == "GET" && r.URL.Path == "/v26.0/123/message_templates" {
					_, _ = io.WriteString(w, `{"data":[{"id":"789"}]}`)
					return
				}
				targetCalls.Add(1)
				if r.Method != tc.method || r.URL.Path != "/v26.0/"+tc.path {
					t.Errorf("unexpected endpoint: %s %s", r.Method, r.URL.Path)
				}
				for name, values := range tc.query {
					if !reflect.DeepEqual(r.URL.Query()[name], values) {
						t.Errorf("unexpected query field %s", name)
					}
				}
				var body map[string]any
				if r.Body != nil {
					_ = json.NewDecoder(r.Body).Decode(&body)
				}
				if !reflect.DeepEqual(body, tc.body) {
					t.Errorf("incorrect body fields: got %v want %v", reflect.ValueOf(body).MapKeys(), reflect.ValueOf(tc.body).MapKeys())
				}
				_, _ = io.WriteString(w, tc.response)
			})
			r := cloudActionsRegistry(t, client, cloudTestConfig())
			registerCloudManagement(r, client)
			result, err := r.Invoke(context.Background(), "whatsapp::cloud::"+tc.operation, tc.input)
			if err != nil {
				t.Fatal(err)
			}
			serialized, _ := json.Marshal(result)
			var wireResult any
			_ = json.Unmarshal(serialized, &wireResult)
			if err := compileDiscoveredCloudSchema(t, entry.ResponseFormat).Validate(wireResult); err != nil {
				t.Fatalf("result violates advertised schema: %v", err)
			}
			if strings.Contains(string(serialized), "access_token") || strings.Contains(string(serialized), "test-access-secret") || strings.Contains(string(serialized), "must-not-return") {
				t.Fatal("management result exposed a credential or unrelated settings")
			}
			if targetCalls.Load() != 1 {
				t.Fatalf("expected one target request, got %d", targetCalls.Load())
			}
		})
	}
}

func TestCloudManagementInvalidRequestsNeverDispatch(t *testing.T) {
	var calls atomic.Int32
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		_, _ = io.WriteString(w, `{"success":true}`)
	})
	r := cloudActionsRegistry(t, client, cloudTestConfig())
	registerCloudManagement(r, client)
	cases := []struct {
		operation string
		input     map[string]any
	}{
		{"templates::get", map[string]any{"template_id": "../me"}},
		{"templates::create", map[string]any{"name": "INVALID", "language": "en", "category": "UTILITY", "components": []any{}}},
		{"templates::update", map[string]any{"template_id": "789"}},
		{"profile::update", map[string]any{}},
		{"profile::update", map[string]any{"websites": []any{"file:///etc/passwd"}}},
		{"phone_numbers::get", map[string]any{"fields": []any{"access_token"}}},
		{"phone_numbers::set_pin", map[string]any{"pin": "123"}},
		{"phone_numbers::register", map[string]any{"pin": "123456", "data_localization_region": "XX"}},
		{"phone_numbers::request_code", map[string]any{"code_method": "EMAIL", "language": "en"}},
		{"subscriptions::subscribe", map[string]any{"override_callback_uri": "http://127.0.0.1/webhook"}},
		{"subscriptions::subscribe", map[string]any{"override_callback_uri": "https://user:pass@example.com/hook"}},
		{"commerce::update", map[string]any{}},
		{"qr_codes::create", map[string]any{"prefilled_message": strings.Repeat("a", 141)}},
		{"qr_codes::delete", map[string]any{"code": "../other"}},
		{"conversational_components::update", map[string]any{"prompts": []any{"😀"}}},
		{"conversational_components::update", map[string]any{"commands": []any{map[string]any{"command_name": "help", "command_description": ""}}}},
		{"blocking::block", map[string]any{"users": []any{"123@g.us"}}},
		{"identity::update", map[string]any{}},
		{"analytics::get", map[string]any{"field": "analytics).access_token", "start": 1, "end": 2, "granularity": "DAY"}},
		{"analytics::get", map[string]any{"field": "analytics", "start": 2, "end": 1, "granularity": "DAY"}},
	}
	for _, tc := range cases {
		t.Run(tc.operation, func(t *testing.T) {
			_, err := r.Invoke(context.Background(), "whatsapp::cloud::"+tc.operation, tc.input)
			assertCloudDocumentError(t, err, "invalid_request")
		})
	}
	// The catalog patterns reject these first; pin the handler's own path guards.
	for _, tc := range []struct{ operation, field string }{{"templates::get", "template_id"}, {"templates::unpause", "template_id"}, {"qr_codes::get", "code"}, {"qr_codes::delete", "code"}} {
		_, err := client.manageCloud(context.Background(), cloudTestConfig(), tc.operation, map[string]any{tc.field: "../other"})
		assertCloudDocumentError(t, err, "invalid_request")
	}
	if calls.Load() != 0 {
		t.Fatal("invalid request reached HTTP transport")
	}
}

func TestCloudManagementAnalyticsAndAccountScope(t *testing.T) {
	now := time.Now().Unix()
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/v26.0/123/message_templates" {
			_, _ = io.WriteString(w, `{"data":[{"id":"888"}]}`)
			return
		}
		if r.URL.Path != "/v26.0/123" {
			t.Error("escaped configured WABA")
		}
		want := fmt.Sprintf("analytics.start(%d).end(%d).granularity(DAY)", now-86400, now)
		if r.URL.Query().Get("fields") != want {
			t.Error("incorrect analytics filter construction")
		}
		_, _ = io.WriteString(w, `{"analytics":{"data_points":[{"sent":2}]},"access_token":"test-access-secret"}`)
	})
	_, err := client.manageCloud(context.Background(), cloudTestConfig(), "analytics::get", map[string]any{"field": "analytics", "start": now - 86400, "end": now, "granularity": "DAY"})
	if err != nil {
		t.Fatal(err)
	}
	_, err = client.manageCloud(context.Background(), cloudTestConfig(), "templates::update", map[string]any{"template_id": "789", "category": "UTILITY"})
	assertCloudDocumentError(t, err, "cloud_resource_not_found")
}

func TestCloudManagementFailuresNoReplay(t *testing.T) {
	for _, method := range []string{"templates::create", "subscriptions::unsubscribe"} {
		t.Run(method, func(t *testing.T) {
			var calls atomic.Int32
			client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
				calls.Add(1)
				select {
				case <-r.Context().Done():
				case <-time.After(100 * time.Millisecond):
				}
			})
			ctx, cancel := context.WithTimeout(context.Background(), 30*time.Millisecond)
			defer cancel()
			input := map[string]any{}
			if method == "templates::create" {
				input = map[string]any{"name": "report_ready", "language": "en", "category": "UTILITY", "components": []any{map[string]any{"type": "BODY", "text": "Ready"}}}
			}
			_, err := client.manageCloud(ctx, cloudTestConfig(), method, input)
			assertCloudDocumentError(t, err, "cloud_outcome_unknown")
			if calls.Load() != 1 {
				t.Fatal("mutation was retried")
			}
		})
	}
	var calls atomic.Int32
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		w.WriteHeader(400)
		_, _ = io.WriteString(w, `{"error":{"code":139100,"message":"test-access-secret"},"block_users":{"added_users":[{"input":"+15551234567"}]}}`)
	})
	_, err := client.manageCloud(context.Background(), cloudTestConfig(), "blocking::block", map[string]any{"users": []any{"+15551234567"}})
	assertCloudDocumentError(t, err, "cloud_api_error")
	config := cloudTestConfig()
	config.Cloud.AccessToken = ""
	_, err = client.manageCloud(context.Background(), config, "phone_numbers::get", nil)
	assertCloudDocumentError(t, err, "cloud_not_configured")
	if calls.Load() != 1 {
		t.Fatal("missing credentials dispatched a request")
	}
}

func TestCloudManagementSanitizesNestedRecordsAndCursors(t *testing.T) {
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		_, _ = io.WriteString(w, `{"data":[{"whatsapp_business_api_data":{"id":"789","name":"test-access-secret","access_token":"test-access-secret","nested":[{"url":"https://example.com?access_token=test-access-secret"}]} }],"paging":{"next":"https://graph.facebook.com?access_token=test-access-secret","cursors":{"after":"prefix-test-access-secret"}}}`)
	})
	result, err := client.manageCloud(context.Background(), cloudTestConfig(), "subscriptions::list", nil)
	if err != nil {
		t.Fatal(err)
	}
	encoded, _ := json.Marshal(result)
	if strings.Contains(string(encoded), "test-access-secret") || strings.Contains(string(encoded), "access_token") || strings.Contains(string(encoded), "graph.facebook.com") {
		t.Fatal("nested management result or cursor exposed credentials")
	}
}

func TestCloudManagementBlocklistCursorWithoutPagingURL(t *testing.T) {
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		_, _ = io.WriteString(w, `{"data":[{"wa_id":"15551234567"}],"paging":{"cursors":{"after":"next-page"}}}`)
	})
	result, err := client.manageCloud(context.Background(), cloudTestConfig(), "blocking::list", nil)
	if err != nil {
		t.Fatal(err)
	}
	if result.(map[string]any)["next_cursor"] != "next-page" {
		t.Fatal("documented blocklist cursor was discarded")
	}
}

func TestCloudManagementRejectsMissingMutationAcknowledgement(t *testing.T) {
	for _, operation := range []string{"qr_codes::create", "templates::create", "blocking::unblock", "commerce::update"} {
		t.Run(operation, func(t *testing.T) {
			var calls atomic.Int32
			client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) { calls.Add(1); _, _ = io.WriteString(w, `{}`) })
			requests := map[string]map[string]any{
				"qr_codes::create":  {"prefilled_message": "Hello"},
				"templates::create": {"name": "report_ready", "language": "en", "category": "UTILITY", "components": []any{map[string]any{"type": "BODY", "text": "Ready"}}},
				"blocking::unblock": {"users": []any{"+15551234567"}},
				"commerce::update":  {"is_cart_enabled": true},
			}
			_, err := client.manageCloud(context.Background(), cloudTestConfig(), operation, requests[operation])
			assertCloudDocumentError(t, err, "cloud_outcome_unknown")
			if calls.Load() != 1 {
				t.Fatal("mutation with ambiguous acknowledgement was retried")
			}
		})
	}
}
