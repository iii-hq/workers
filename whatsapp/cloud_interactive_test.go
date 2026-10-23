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

func TestCloudInteractiveHTTP(t *testing.T) {
	var last map[string]any
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		if r.Method != "POST" || r.URL.Path != "/v26.0/456/messages" || r.Header.Get("Authorization") != "Bearer test-access-secret" {
			t.Error("unexpected endpoint or auth")
		}
		if err := json.NewDecoder(r.Body).Decode(&last); err != nil {
			t.Error(err)
		}
		fmt.Fprint(w, `{"messages":[{"id":"wamid.interactive","message_status":"accepted"}]}`)
	})
	registry := cloudActionsRegistry(t, client, cloudTestConfig())
	tests := []struct {
		name, kind    string
		input, action map[string]any
	}{
		{"send_list", "list", map[string]any{"button": "Choose", "sections": []any{map[string]any{"title": "Delivery", "rows": []any{map[string]any{"id": "express", "title": "Express", "description": "Tomorrow"}}}}}, map[string]any{"button": "Choose", "sections": []any{map[string]any{"title": "Delivery", "rows": []any{map[string]any{"id": "express", "title": "Express", "description": "Tomorrow"}}}}}},
		{"send_buttons", "button", map[string]any{"buttons": []any{map[string]any{"id": "yes", "title": "Yes"}}}, map[string]any{"buttons": []any{map[string]any{"type": "reply", "reply": map[string]any{"id": "yes", "title": "Yes"}}}}},
		{"send_cta_url", "cta_url", map[string]any{"display_text": "Open", "url": "https://example.com/report"}, map[string]any{"name": "cta_url", "parameters": map[string]any{"display_text": "Open", "url": "https://example.com/report"}}},
		{"request_location", "location_request_message", map[string]any{}, map[string]any{"name": "send_location"}},
		{"send_flow", "flow", map[string]any{"flow_id": "123", "flow_cta": "Book", "screen": "START", "data": map[string]any{"date": "2026-10-02"}}, map[string]any{"name": "flow", "parameters": map[string]any{"flow_message_version": "3", "flow_id": "123", "flow_cta": "Book", "mode": "published", "flow_action": "navigate", "flow_action_payload": map[string]any{"screen": "START", "data": map[string]any{"date": "2026-10-02"}}}}},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			tc.input["chat_id"] = "+15551234567"
			tc.input["text"] = "Choose an option"
			tc.input["reply_to_message_id"] = "wamid.original"
			got, err := registry.Invoke(context.Background(), "whatsapp::cloud::messages::"+tc.name, tc.input)
			if err != nil {
				t.Fatal(err)
			}
			if got.(CloudSentMessage).Status != "accepted" {
				t.Fatal("must return acceptance only")
			}
			interactive := last["interactive"].(map[string]any)
			if last["type"] != "interactive" || interactive["type"] != tc.kind || !reflect.DeepEqual(interactive["action"], tc.action) || !reflect.DeepEqual(last["context"], map[string]any{"message_id": "wamid.original"}) {
				t.Fatalf("bad %s payload: %#v", tc.name, last)
			}
		})
	}
	cards := []any{}
	for _, id := range []string{"100", "200"} {
		cards = append(cards, map[string]any{"header": map[string]any{"type": "image", "media_id": id}, "text": "Card", "buttons": []any{map[string]any{"id": id, "title": "Choose"}}})
	}
	_, err := registry.Invoke(context.Background(), "whatsapp::cloud::messages::send_carousel", map[string]any{"chat_id": "+15551234567", "text": "Browse", "cards": cards})
	if err != nil {
		t.Fatal(err)
	}
	interactive := last["interactive"].(map[string]any)
	sentCards := interactive["action"].(map[string]any)["cards"].([]any)
	for i, item := range sentCards {
		card := item.(map[string]any)
		if card["type"] != "cta_url" || card["card_index"] != float64(i) {
			t.Fatalf("wrong card layout: %#v", card)
		}
		button := card["action"].(map[string]any)["buttons"].([]any)[0].(map[string]any)
		if button["type"] != "quick_reply" || button["quick_reply"] == nil {
			t.Fatal("carousel requires quick_reply rather than reply")
		}
	}
	_, err = registry.Invoke(context.Background(), "whatsapp::cloud::messages::send_buttons", map[string]any{"chat_id": "+15551234567", "text": "Report", "header": map[string]any{"type": "document", "media_id": "123"}, "buttons": []any{map[string]any{"id": "ok", "title": "OK"}}})
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(last["interactive"].(map[string]any)["header"], map[string]any{"type": "document", "document": map[string]any{"id": "123"}}) {
		t.Fatal("media header not mapped to ID")
	}
}

func TestCloudInteractiveValidationBeforeHTTP(t *testing.T) {
	var calls atomic.Int32
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) { calls.Add(1) })
	registry := cloudActionsRegistry(t, client, cloudTestConfig())
	button := map[string]any{"id": "one", "title": "One"}
	tests := []struct {
		name  string
		input map[string]any
	}{
		{"send_buttons", map[string]any{"buttons": []any{button, button}}},
		{"send_buttons", map[string]any{"buttons": []any{map[string]any{"id": "one", "title": strings.Repeat("é", 21)}}}},
		{"send_buttons", map[string]any{"buttons": []any{button}, "header": map[string]any{"type": "image", "link": "http://localhost/private"}}},
		{"send_buttons", map[string]any{"buttons": []any{button}, "text": strings.Repeat("x", 1025)}},
		{"send_list", map[string]any{"button": "Choose", "sections": []any{map[string]any{"title": "A", "rows": []any{map[string]any{"id": "x", "title": "A"}, map[string]any{"id": "x", "title": "B"}}}}}},
		{"send_list", map[string]any{"button": "Choose", "sections": []any{}}},
		{"send_cta_url", map[string]any{"display_text": "Open", "url": "file:///etc/passwd"}},
		{"send_cta_url", map[string]any{"display_text": "Open", "url": "https://user:password@example.com"}},
		{"request_location", map[string]any{"footer": "Unsupported"}},
		{"send_carousel", map[string]any{"cards": []any{}}},
		{"send_carousel", map[string]any{"cards": []any{map[string]any{"header": map[string]any{"type": "image", "media_id": "1"}, "buttons": []any{button}}, map[string]any{"header": map[string]any{"type": "image", "media_id": "2"}, "display_text": "Open", "url": "https://example.com"}}}},
		{"send_flow", map[string]any{"flow_id": "1", "flow_name": "also", "flow_cta": "Book", "screen": "START"}},
		{"send_flow", map[string]any{"flow_id": "1", "flow_cta": "Book"}},
		{"send_flow", map[string]any{"flow_id": "1", "flow_cta": "Book", "flow_action": "data_exchange", "screen": "START"}},
		{"send_flow", map[string]any{"flow_id": "1", "flow_cta": "Book", "screen": "START", "data": map[string]any{}}},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			tc.input["chat_id"] = "+15551234567"
			if _, ok := tc.input["text"]; !ok {
				tc.input["text"] = "Hello"
			}
			_, err := registry.Invoke(context.Background(), "whatsapp::cloud::messages::"+tc.name, tc.input)
			assertCloudDocumentError(t, err, "invalid_request")
		})
	}
	if calls.Load() != 0 {
		t.Fatalf("invalid requests reached Meta %d times", calls.Load())
	}
}

func TestCloudInteractiveListCurrentTextLimit(t *testing.T) {
	req := CloudListRequest{CloudInteractiveBase: CloudInteractiveBase{ChatID: "+15551234567", Text: strings.Repeat("é", 4096)}, Button: "Choose", Sections: []CloudListSection{{Title: "Options", Rows: []CloudListRow{{ID: "1", Title: "One"}}}}}
	if _, err := cloudListPayload(req); err != nil {
		t.Fatalf("current list docs allow 4096 characters: %v", err)
	}
	req.Text += "x"
	if _, err := cloudListPayload(req); err == nil {
		t.Fatal("oversized list body accepted")
	}
}
