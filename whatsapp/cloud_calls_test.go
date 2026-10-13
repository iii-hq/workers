package main

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"strings"
	"testing"
)

const cloudTestSDP = "v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=rtpmap:111 opus/48000/2\r\n"

func TestCloudCallsSignalHTTP(t *testing.T) {
	for _, action := range []string{"connect", "pre_accept", "accept", "reject", "terminate"} {
		t.Run(action, func(t *testing.T) {
			calls := 0
			client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
				calls++
				if r.Method != "POST" || r.URL.Path != "/v26.0/456/calls" || r.Header.Get("Authorization") != "Bearer test-access-secret" {
					t.Error("incorrect call route/auth")
				}
				var body map[string]any
				if json.NewDecoder(r.Body).Decode(&body) != nil {
					t.Error("invalid JSON")
				}
				if body["action"] != action || body["messaging_product"] != "whatsapp" {
					t.Errorf("incorrect call payload: %#v", body)
				}
				if action == "connect" {
					if body["recipient"] != "US.abc123" || body["to"] != nil {
						t.Error("incorrect BSUID call routing")
					}
					_, _ = io.WriteString(w, `{"calls":[{"id":"wacid.example"}]}`)
				} else {
					if body["call_id"] != "wacid.example" {
						t.Error("missing call ID")
					}
					_, _ = io.WriteString(w, `{"success":true}`)
				}
			})
			input := map[string]any{}
			if action == "connect" {
				input["chat_id"] = "US.abc123"
				input["session"] = map[string]any{"sdp_type": "offer", "sdp": cloudTestSDP}
			} else {
				input["call_id"] = "wacid.example"
				if action == "pre_accept" {
					input["session"] = map[string]any{"sdp_type": "answer", "sdp": cloudTestSDP}
				}
			}
			result, err := client.signalCloudCall(context.Background(), cloudTestConfig(), action, input)
			if err != nil || result.CallID != "wacid.example" || result.Status != "accepted" || calls != 1 {
				t.Fatalf("result=%+v error=%v calls=%d", result, err, calls)
			}
		})
	}
}

func TestCloudCallsSettingsAndPermissions(t *testing.T) {
	var routes []string
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		routes = append(routes, r.Method+" "+r.URL.Path)
		if r.URL.Query().Get("include_sip_credentials") != "" {
			t.Error("SIP secrets requested")
		}
		if r.URL.Path == "/v26.0/456/call_permissions" && r.URL.Query().Get("recipient") != "US.abc123" {
			t.Error("permissions must support BSUID")
		}
		_, _ = io.WriteString(w, `{"calling":{"status":"ENABLED","sip":{"servers":[{"sip_user_password":"private-sip-secret"}]}},"permission":{"status":"temporary"},"success":true}`)
	})
	for _, tt := range []struct {
		operation string
		input     map[string]any
	}{
		{"settings::get", map[string]any{}},
		{"settings::update", map[string]any{"calling": map[string]any{"status": "ENABLED", "call_icon_visibility": "DEFAULT", "audio": map[string]any{"additional_codecs": []any{"PCMA"}}}}},
		{"permissions::get", map[string]any{"chat_id": "US.abc123"}},
	} {
		result, err := client.manageCloudCall(context.Background(), cloudTestConfig(), tt.operation, tt.input)
		if err != nil {
			t.Fatal(err)
		}
		raw, _ := json.Marshal(result)
		if strings.Contains(string(raw), "private-sip-secret") || strings.Contains(string(raw), "sip_user_password") {
			t.Error("SIP credential leaked")
		}
	}
	if len(routes) != 3 {
		t.Fatalf("unexpected request count: %v", routes)
	}
}

func TestCloudCallsValidationAndSafeErrors(t *testing.T) {
	client := cloudTestClient(t, func(http.ResponseWriter, *http.Request) { t.Error("invalid call reached Meta") })
	r := cloudActionsRegistry(t, client, cloudTestConfig())
	registerCloudCalls(r, client)
	for _, tt := range []struct {
		operation string
		input     map[string]any
	}{
		{"connect", map[string]any{"chat_id": "123@g.us", "session": map[string]any{"sdp_type": "offer", "sdp": cloudTestSDP}}},
		{"connect", map[string]any{"chat_id": "+15551234567"}},
		{"pre_accept", map[string]any{"call_id": "wacid.example", "session": map[string]any{"sdp_type": "offer", "sdp": cloudTestSDP}}},
		{"reject", map[string]any{"call_id": "wacid.example", "session": map[string]any{"sdp_type": "answer", "sdp": cloudTestSDP}}},
		{"terminate", map[string]any{"call_id": "wacid.bad\nID"}},
	} {
		t.Run(tt.operation, func(t *testing.T) {
			_, err := r.Invoke(context.Background(), "whatsapp::cloud::calls::"+tt.operation, tt.input)
			assertCloudDocumentError(t, err, "invalid_request")
		})
	}
	for _, tt := range []struct {
		response string
		status   int
		code     string
	}{
		{`{"error":{"code":100,"message":"test-access-secret"}}`, 400, "cloud_api_error"},
		{`{"calls":[]}`, 200, "cloud_outcome_unknown"},
		{`{"calls":[{"id":"wacid.test-access-secret"}]}`, 200, "cloud_outcome_unknown"},
		{`test-access-secret`, 200, "cloud_outcome_unknown"},
	} {
		calls := 0
		client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
			calls++
			w.WriteHeader(tt.status)
			_, _ = io.WriteString(w, tt.response)
		})
		_, err := client.signalCloudCall(context.Background(), cloudTestConfig(), "connect", map[string]any{"chat_id": "+15551234567", "session": map[string]any{"sdp_type": "offer", "sdp": cloudTestSDP}})
		assertCloudDocumentError(t, err, tt.code)
		if calls != 1 {
			t.Error("call retried")
		}
	}
}

func TestCloudCallsSettingsValidationAndPreciseMediaID(t *testing.T) {
	client := cloudTestClient(t, func(http.ResponseWriter, *http.Request) { t.Error("invalid settings reached Meta") })
	r := cloudActionsRegistry(t, client, cloudTestConfig())
	registerCloudCalls(r, client)
	for _, input := range []map[string]any{
		{"calling": map[string]any{}},
		{"calling": map[string]any{"sip": map[string]any{"servers": []any{map[string]any{"hostname": "127.0.0.1"}}}}},
		{"calling": map[string]any{"audio": map[string]any{"additional_codecs": []any{"MP3"}}}},
		{"calling": map[string]any{"call_hours": map[string]any{"status": "ENABLED", "timezone_id": "not/a/timezone", "weekly_operating_hours": []any{map[string]any{"day_of_week": "MONDAY", "open_time": "0900", "close_time": "1800"}}}}},
		{"calling": map[string]any{"voicemail": map[string]any{"status": "ENABLED", "triggers": []any{"TIMEOUT"}, "audio": map[string]any{"default": map[string]any{"announcement_media_id": "987"}}}}},
	} {
		_, err := r.Invoke(context.Background(), "whatsapp::cloud::calls::settings::update", input)
		assertCloudDocumentError(t, err, "invalid_request")
	}
	timeout := 20
	body, err := cloudCallSettingsBody(CloudCallSettingsRequest{Calling: CloudCallingSettings{Voicemail: &CloudVoicemail{Status: "ENABLED", Triggers: []string{"TIMEOUT"}, Audio: &CloudVoicemailAudio{Default: CloudVoicemailDefault{AnnouncementMediaID: "9876543210123456789", TimeoutSeconds: &timeout}}}}})
	if err != nil {
		t.Fatal(err)
	}
	raw, _ := json.Marshal(body)
	if !strings.Contains(string(raw), `"announcement_media_id":9876543210123456789`) {
		t.Fatalf("media ID lost precision: %s", raw)
	}
}

func TestCloudCallsPermissionRequestHTTP(t *testing.T) {
	calls := 0
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		calls++
		var body map[string]any
		if json.NewDecoder(r.Body).Decode(&body) != nil {
			t.Error("invalid request JSON")
		}
		interactive, _ := body["interactive"].(map[string]any)
		action, _ := interactive["action"].(map[string]any)
		if body["type"] != "interactive" || interactive["type"] != "call_permission_request" || action["name"] != "call_permission_request" || body["to"] != "+15551234567" {
			t.Errorf("wrong permission request: %#v", body)
		}
		_, _ = io.WriteString(w, `{"messages":[{"id":"wamid.permission"}]}`)
	})
	registry := cloudTestRegistry(t, client, cloudTestConfig(), registerCloudCalls)
	result, err := registry.Invoke(context.Background(), "whatsapp::cloud::calls::permissions::request", map[string]any{"chat_id": "+15551234567", "text": "May we call you?"})
	if err != nil || result.(CloudSentMessage).Status != "accepted" || calls != 1 {
		t.Fatalf("result=%+v error=%v", result, err)
	}
}
