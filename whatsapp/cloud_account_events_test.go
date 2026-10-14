package main

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestCloudAccountWebhookRequiresOptInAndDeduplicates(t *testing.T) {
	payload := cloudTestPayload(map[string]any{"field": "message_template_status_update", "value": map[string]any{"event": "APPROVED", "message_template_id": "999", "message_template_name": "report"}})
	payload["entry"].([]any)[0].(map[string]any)["time"] = float64(1790870400)
	batch := cloudTestParse(t, payload)
	if len(batch.events) != 1 || len(batch.messages) != 0 {
		t.Fatalf("expected one administrative event, got %+v", batch)
	}
	defaults, err := parseCloudEventFilter(json.RawMessage(`{}`))
	if err != nil {
		t.Fatal(err)
	}
	if defaults.matches(batch.events[0]) {
		t.Fatal("existing default subscription received administrative event")
	}
	filter, err := parseCloudEventFilter(json.RawMessage(`{"kinds":["webhook.received"],"fields":["message_template_status_update"],"events":["APPROVED"]}`))
	if err != nil {
		t.Fatal(err)
	}
	if !filter.matches(batch.events[0]) {
		t.Fatal("explicit filter did not match")
	}
	inbox := inboxTestOpen(t, 100)
	first, err := inbox.RecordCloudDelivery(nil, nil, batch.events)
	if err != nil || len(first) != 1 {
		t.Fatalf("store: %v %+v", err, first)
	}
	second, err := inbox.RecordCloudDelivery(nil, nil, batch.events)
	if err != nil || len(second) != 0 {
		t.Fatalf("duplicate delivered: %v %+v", err, second)
	}
	raw, _ := json.Marshal(batch.events[0])
	var value any
	_ = json.Unmarshal(raw, &value)
	if err := compileCloudEventSchema(t, cloudEventPayloadSchema).Validate(value); err != nil {
		t.Fatal(err)
	}
}

func TestCloudAdministrativeEventBoundIncludesRoutingFields(t *testing.T) {
	config := cloudTestConfig().Cloud
	value := map[string]any{"groups": []any{map[string]any{"group_id": strings.Repeat("g", 256), "type": "group_create"}}, "details": ""}
	event, err := cloudAccountEvent(config, map[string]any{}, value, "group_lifecycle_update")
	if err != nil {
		t.Fatal(err)
	}
	empty, _ := json.Marshal(event)
	value["details"] = strings.Repeat("x", 64*1024-len(empty)+20)
	event, err = cloudAccountEvent(config, map[string]any{}, value, "group_lifecycle_update")
	if err != nil {
		t.Fatal(err)
	}
	encoded, _ := json.Marshal(event)
	if len(encoded) > 64*1024 || string(event.Data) != `{"content_omitted":true}` || event.GroupID == "" {
		t.Fatal("Administrative routing fields bypassed event byte limit")
	}
}

func TestCloudAccountWebhookScopesPhoneAndSanitizesCredentials(t *testing.T) {
	config := cloudTestConfig()
	for _, phone := range []string{"456", "987"} {
		payload := cloudTestPayload(map[string]any{"field": "calls", "value": map[string]any{"metadata": map[string]any{"phone_number_id": phone}, "calls": []any{map[string]any{"id": "call123"}}, "access_token": config.Cloud.AccessToken}})
		raw, _ := json.Marshal(payload)
		batch, err := parseCloudDelivery(config.Cloud, raw)
		if err != nil {
			t.Fatal(err)
		}
		if phone == "987" {
			if len(batch.events) != 0 {
				t.Fatal("other phone leaked")
			}
			continue
		}
		if len(batch.events) != 1 {
			t.Fatal("call event missing")
		}
		var data map[string]any
		_ = json.Unmarshal(batch.events[0].Data, &data)
		if _, exists := data["access_token"]; exists {
			t.Fatal("credential field exposed")
		}
	}
}
