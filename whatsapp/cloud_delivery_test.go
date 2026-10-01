package main

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
)

func TestCloudDeliveryPreservesInteractivePayloadAndProviderTimes(t *testing.T) {
	message := cloudTestMessage("interactive")
	message["type"] = "interactive"
	message["interactive"] = map[string]any{"type": "button_reply", "button_reply": map[string]any{"id": "confirm_order", "title": "Confirm"}}
	delete(message, "text")
	change := cloudTestChange(message)
	change["value"].(map[string]any)["statuses"] = []any{map[string]any{"id": "outgoing", "status": "read", "recipient_id": "15551234567", "timestamp": "1700000005"}}
	batch := cloudTestParse(t, cloudTestPayload(change))
	if len(batch.events) != 2 || batch.messages[0].Kind != "text" {
		t.Fatalf("wrong delivery: %+v", batch)
	}
	incoming, status := batch.events[0], batch.events[1]
	if incoming.MessageType != "interactive" || incoming.SenderID != "+15551234567" || incoming.Text != "Confirm" || incoming.TimestampMS != 1700000000000 {
		t.Fatalf("message projection lost its provider fields: %+v", incoming)
	}
	var data map[string]any
	if err := json.Unmarshal(incoming.Data, &data); err != nil {
		t.Fatal(err)
	}
	if cloudString(cloudNested(cloudNested(data, "interactive"), "button_reply"), "id") != "confirm_order" {
		t.Fatal("interactive reply ID was dropped")
	}
	if status.Kind != "message.status" || status.Status != "read" || status.TimestampMS != 1700000005000 || status.SenderID != "" || status.MessageType != "" {
		t.Fatalf("status must retain its event time and recipient: %+v", status)
	}
}

func TestCloudDeliveryOmitsOversizedDataWithoutLosingRoutingFields(t *testing.T) {
	message := cloudTestMessage("large")
	message["text"] = map[string]any{"body": strings.Repeat("\x01", 20000)}
	batch := cloudTestParse(t, cloudTestPayload(cloudTestChange(message)))
	event := batch.events[0]
	encoded, err := json.Marshal(event)
	if err != nil || len(encoded) > 64*1024 {
		t.Fatalf("oversized event: %d bytes, %v", len(encoded), err)
	}
	if string(event.Data) != `{"content_omitted":true}` || event.MessageType != "text" || event.ChatID != "+15551234567" || event.MessageID != "large" {
		t.Fatal("bounded event lost routing fields or retained oversized data")
	}
}

func TestCloudDeliveryQueuesOnlyCommittedCurrentAccountEvents(t *testing.T) {
	config := cloudTestConfig()
	config.DataDir = t.TempDir()
	manager, err := NewManager(config)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = manager.Shutdown(context.Background()) })
	batch := cloudTestParse(t, cloudTestPayload(cloudTestChange(cloudTestMessage("queued"))))
	if err = manager.storeCloudDelivery(config, batch); err != nil {
		t.Fatal(err)
	}
	event := <-manager.CloudEventQueue
	if !manager.cloudEventCurrent(event) {
		t.Fatal("fresh account event rejected")
	}
	inbox, err := manager.CloudInbox(config)
	if err != nil {
		t.Fatal(err)
	}
	page, err := inbox.ListMessages("cloud:+15551234567", "", 10)
	if err != nil || len(page.Messages) != 1 {
		t.Fatal("event became available before the archive commit")
	}
	next := config
	next.Cloud.PhoneNumberID = "789"
	if _, err = manager.ApplyConfig(next); err != nil {
		t.Fatal(err)
	}
	if err = manager.storeCloudDelivery(config, batch); err == nil {
		t.Fatal("stale webhook configuration was accepted")
	}
	if _, err = manager.ApplyConfig(config); err != nil {
		t.Fatal(err)
	}
	if manager.cloudEventCurrent(event) {
		t.Fatal("switching accounts back revived a stale queued event")
	}
	if _, err = inbox.db.Exec(`CREATE TRIGGER reject_delivery BEFORE INSERT ON inbox_cloud_event_markers BEGIN SELECT RAISE(ABORT, 'test failure'); END;`); err != nil {
		t.Fatal(err)
	}
	failed := cloudTestParse(t, cloudTestPayload(cloudTestChange(cloudTestMessage("failed"))))
	if err = manager.storeCloudDelivery(config, failed); err == nil || len(manager.CloudEventQueue) != 0 {
		t.Fatal("failed transaction published an event")
	}
}

func TestCloudDeliveryFullQueueKeepsArchiveAvailable(t *testing.T) {
	config := cloudTestConfig()
	config.DataDir = t.TempDir()
	config.MaxEvents = 1
	manager, err := NewManager(config)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = manager.Shutdown(context.Background()) })
	batch := cloudTestParse(t, cloudTestPayload(cloudTestChange(cloudTestMessage("one"), cloudTestMessage("two"))))
	if err = manager.storeCloudDelivery(config, batch); err != nil {
		t.Fatal(err)
	}
	inbox, err := manager.CloudInbox(config)
	if err != nil {
		t.Fatal(err)
	}
	page, err := inbox.ListMessages("cloud:+15551234567", "", 10)
	if err != nil || len(page.Messages) != 2 || len(manager.CloudEventQueue) != 1 {
		t.Fatal("queue pressure blocked archival or exceeded its limit")
	}
}
