package main

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"net/http"
	"strings"
	"sync/atomic"
	"testing"
)

func TestCloudExistingSendAcceptsContextualReply(t *testing.T) {
	var req CloudTextRequest
	if err := decodeCloudRequest(map[string]any{"chat_id": "+15551234567", "text": "Reply", "reply_to_message_id": "wamid.original"}, &req); err != nil {
		t.Fatal(err)
	}
	body, err := cloudTextPayload(req)
	if err != nil {
		t.Fatal(err)
	}
	context, _ := body["context"].(map[string]any)
	if context["message_id"] != "wamid.original" {
		t.Fatalf("missing contextual reply: %#v", body)
	}
}

func TestCloudDeleteTimeoutHasUnknownOutcome(t *testing.T) {
	var calls atomic.Int32
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte(`not-json`))
	})
	_, err := client.request(context.Background(), cloudTestConfig(), http.MethodDelete, "987", nil, nil)
	assertCloudDocumentError(t, err, "cloud_outcome_unknown")
	if calls.Load() != 1 {
		t.Fatal("deletion retried")
	}
}

func TestCloudSanitizeResultDoesNotExposeSecretsOrPagingURLs(t *testing.T) {
	config := cloudTestConfig()
	value := map[string]any{"id": "123", "access_token": config.Cloud.AccessToken, "paging": map[string]any{"next": "https://graph.facebook.com/?access_token=" + config.Cloud.AccessToken}, "data": []any{map[string]any{"text": "prefix " + config.Cloud.AccessToken, "safe": "ok"}}}
	cleaned := cloudSanitizeResult(config, value)
	raw, err := json.Marshal(cleaned)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(raw), config.Cloud.AccessToken) || strings.Contains(string(raw), "access_token") || strings.Contains(string(raw), "graph.facebook.com") {
		t.Fatalf("unsafe result: %s", raw)
	}
	if !strings.Contains(string(raw), `"safe":"ok"`) {
		t.Fatal("safe data lost")
	}
}

func TestCloudSanitizeTypedContainersAndCredentialEncodings(t *testing.T) {
	config := cloudTestConfig()
	cursor := "prefix-" + base64.StdEncoding.EncodeToString([]byte(config.Cloud.AccessToken))
	value := map[string]any{
		"items":    []map[string]any{{"SIP_USER_PASSWORD": "remote-credential", "sip_username": "sip-account", "clientSecret": "remote-credential", "name": "keep this", "flow_token": "application-session"}},
		"cursor":   &cursor,
		"values":   []string{base64.RawURLEncoding.EncodeToString([]byte(config.Cloud.AppSecret))},
		"raw":      json.RawMessage(`{"accessToken":"remote-credential","ok":true}`),
		"url":      "https://example.com/path?client_secret=remote-credential&safe=1",
		"userinfo": "https://user:remote-credential@example.com/path",
		"next":     "CONFIRMATION_SCREEN",
		"previous": map[string]any{"screen": "ADDRESS"},
	}
	raw, _ := json.Marshal(cloudSanitizeResult(config, value))
	for _, secret := range []string{"remote-credential", config.Cloud.AccessToken, base64.StdEncoding.EncodeToString([]byte(config.Cloud.AccessToken)), base64.RawURLEncoding.EncodeToString([]byte(config.Cloud.AppSecret))} {
		if strings.Contains(string(raw), secret) {
			t.Fatal("encoded credential escaped recursive sanitizer")
		}
	}
	if !strings.Contains(string(raw), "keep this") || !strings.Contains(string(raw), "application-session") {
		t.Fatal("non-credential business content was dropped")
	}
	if !strings.Contains(string(raw), "CONFIRMATION_SCREEN") || !strings.Contains(string(raw), "ADDRESS") {
		t.Fatal("business next/previous data was mistaken for pagination URLs")
	}
	config.Cloud.VerifyToken = "a"
	result := cloudSanitizeResult(config, map[string]any{"name": "data available", "id": "12345", "credential": "a"}).(map[string]any)
	if result["name"] != "data available" || result["id"] != "12345" || result["credential"] != "[redacted]" {
		t.Fatal("short verify token corrupted unrelated content")
	}
}

func TestCloudLongCredentialEchoesInOpaqueIdentifiersAreRejected(t *testing.T) {
	config := cloudTestConfig()
	for _, encoded := range []string{config.Cloud.AccessToken, base64.StdEncoding.EncodeToString([]byte(config.Cloud.AccessToken)), base64.RawURLEncoding.EncodeToString([]byte(config.Cloud.AppSecret))} {
		client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
			if strings.HasSuffix(r.URL.Path, "/calls") {
				_, _ = w.Write([]byte(`{"calls":[{"id":"wacid.` + encoded + `"}]}`))
				return
			}
			_, _ = w.Write([]byte(`{"messages":[{"id":"wamid.` + encoded + `"}]}`))
		})
		body, err := cloudTextPayload(CloudTextRequest{ChatID: "+15551234567", Text: "Test"})
		if err != nil {
			t.Fatal(err)
		}
		_, err = client.send(context.Background(), config, "+15551234567", body)
		assertCloudDocumentError(t, err, "cloud_outcome_unknown")
		_, err = client.signalCloudCall(context.Background(), config, "connect", map[string]any{"chat_id": "+15551234567", "session": map[string]any{"sdp_type": "offer", "sdp": "v=0\r\n"}})
		assertCloudDocumentError(t, err, "cloud_outcome_unknown")
	}
	config.Cloud.VerifyToken = "wa"
	if cloudEchoesSecret(config, "wamid.legitimate-wa-message") {
		t.Fatal("short verify token rejected an unrelated opaque identifier")
	}
}
