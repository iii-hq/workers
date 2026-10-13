package main

import (
	"encoding/json"
	"testing"
)

func TestCloudGroupWebhookUsesGroupConversationAndParticipant(t *testing.T) {
	group := "Y2FwaV9ncm91cDoxMjM="
	change := cloudTestChange(map[string]any{"id": "wamid.group", "timestamp": "1790870400", "from": "15551234567", "group_id": group, "type": "text", "text": map[string]any{"body": "Group message"}})
	batch := cloudTestParse(t, cloudTestPayload(change))
	if len(batch.messages) != 1 || batch.messages[0].ChatID != "cloud:group:"+group || batch.messages[0].SenderID != "+15551234567" {
		t.Fatalf("wrong group route: %+v", batch)
	}
	raw, _ := json.Marshal(batch.events[0])
	var value map[string]any
	_ = json.Unmarshal(raw, &value)
	if value["group_id"] != group || value["chat_id"] != "group:"+group {
		t.Fatal("group event identity lost")
	}
	filter, err := parseCloudEventFilter(json.RawMessage(`{"group_ids":["` + group + `"]}`))
	if err != nil || !filter.matches(batch.events[0]) {
		t.Fatalf("group filter: %v", err)
	}
	if err := compileCloudEventSchema(t, cloudEventPayloadSchema).Validate(value); err != nil {
		t.Fatal(err)
	}
	inbox := inboxTestOpen(t, 100)
	if _, err = inbox.RecordCloudDelivery(batch.messages, nil, batch.events); err != nil {
		t.Fatal(err)
	}
	page, err := inbox.ListChats("", "", 10)
	if err != nil || len(page.Chats) != 1 || !page.Chats[0].IsGroup || page.Chats[0].Title == "Test sender" {
		t.Fatalf("Cloud group misclassified as contact: %+v %v", page, err)
	}
	if _, err := cloudRecipient("group:" + group); err == nil {
		t.Fatal("individual sender accepted group")
	}
}

func TestCloudGroupDeliveryStatusKeepsGroupRecipient(t *testing.T) {
	group := "Y2FwaV9ncm91cDoxMjM="
	change := cloudTestChange()
	delete(change["value"].(map[string]any), "messages")
	change["value"].(map[string]any)["statuses"] = []any{map[string]any{"id": "wamid.group", "timestamp": "1790870400", "status": "sent", "recipient_type": "group", "recipient_id": group}}
	batch := cloudTestParse(t, cloudTestPayload(change))
	if len(batch.receipts) != 1 || batch.receipts[0].ChatID != "cloud:group:"+group {
		t.Fatalf("group status missing: %+v", batch)
	}
}

func TestCloudGroupParticipantReceiptsRemainDistinct(t *testing.T) {
	group := "Y2FwaV9ncm91cDoxMjM="
	change := cloudTestChange()
	value := change["value"].(map[string]any)
	delete(value, "messages")
	value["statuses"] = []any{
		map[string]any{"id": "wamid.group", "timestamp": "1790870400", "status": "read", "recipient_type": "group", "recipient_id": group, "recipient_participant_id": "15551234567"},
		map[string]any{"id": "wamid.group", "timestamp": "1790870400", "status": "read", "recipient_type": "group", "recipient_id": group, "participant_recipient_id": "15551234568"},
	}
	batch := cloudTestParse(t, cloudTestPayload(change))
	if len(batch.events) != 2 || len(batch.receipts) != 0 {
		t.Fatal("participant receipt treated as group-wide acknowledgement")
	}
	inbox := inboxTestOpen(t, 100)
	fresh, err := inbox.RecordCloudDelivery(nil, nil, batch.events)
	if err != nil || len(fresh) != 2 {
		t.Fatalf("participants collapsed: %v %+v", err, fresh)
	}
	filter, err := parseCloudEventFilter(json.RawMessage(`{"kinds":["message.status"],"participant_ids":["15551234568"]}`))
	if err != nil || filter.matches(fresh[0]) || !filter.matches(fresh[1]) {
		t.Fatalf("participant filter: %v", err)
	}
}

func TestCloudGroupMetadataSplitsAndFiltersGroups(t *testing.T) {
	change := map[string]any{"field": "group_lifecycle_update", "value": map[string]any{"metadata": map[string]any{"phone_number_id": "456"}, "groups": []any{
		map[string]any{"group_id": "GroupOne", "type": "group_create", "timestamp": "1790870400"},
		map[string]any{"group_id": "GroupTwo", "type": "group_create", "timestamp": "1790870400"},
	}}}
	batch := cloudTestParse(t, cloudTestPayload(change))
	if len(batch.events) != 2 {
		t.Fatal("group metadata was not split")
	}
	filter, err := parseCloudEventFilter(json.RawMessage(`{"kinds":["webhook.received"],"group_ids":["GroupTwo"],"events":["group_create"]}`))
	if err != nil || filter.matches(batch.events[0]) || !filter.matches(batch.events[1]) {
		t.Fatalf("metadata filter: %v", err)
	}
}
