package main

import (
	"encoding/json"
	"fmt"
	"testing"
)

func TestCloudWebhookBoundsIncludeGroupParticipantAndAccountEvents(t *testing.T) {
	for _, count := range []int{1000, 1001} {
		statuses := make([]any, count)
		for i := range statuses {
			statuses[i] = map[string]any{"id": fmt.Sprintf("wamid.%d", i), "status": "read", "timestamp": "1790899200", "recipient_type": "group", "recipient_id": "Group123", "recipient_participant_id": "15551234567"}
		}
		change := map[string]any{"field": "messages", "value": map[string]any{"messaging_product": "whatsapp", "metadata": map[string]any{"phone_number_id": "456"}, "statuses": statuses}}
		encoded, _ := json.Marshal(cloudTestPayload(change))
		batch, err := parseCloudDelivery(cloudTestConfig().Cloud, encoded)
		if count == 1000 && (err != nil || len(batch.events) != count || len(batch.receipts) != 0) {
			t.Fatalf("participant receipt boundary rejected: events=%d receipts=%d err=%v", len(batch.events), len(batch.receipts), err)
		}
		if count == 1001 && err == nil {
			t.Fatal("participant status events bypassed webhook event limit")
		}
		if count == 1000 {
			admin := map[string]any{"field": "message_template_status_update", "value": map[string]any{"event": "APPROVED", "message_template_id": "789"}}
			encoded, _ = json.Marshal(cloudTestPayload(admin, change))
			if _, err = parseCloudDelivery(cloudTestConfig().Cloud, encoded); err == nil {
				t.Fatal("mixed administrative/status delivery bypassed total event limit")
			}
		}
	}
}
