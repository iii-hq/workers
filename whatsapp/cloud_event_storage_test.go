package main

import (
	"encoding/json"
	"fmt"
	"sync"
	"testing"
)

func cloudStorageMessage(id string) MessageView {
	return MessageView{MessageID: id, ChatID: "cloud:+15551234567", SenderID: "+15551234567", TimestampMS: 1700000000000, Kind: "text", Text: "Incoming message", Source: "live"}
}

func cloudStorageEvent(id string) CloudEvent {
	return CloudEvent{BusinessAccountID: "987654321", PhoneNumberID: "123456789", Kind: "message.received", MessageID: id, ChatID: "+15551234567", SenderID: "+15551234567", MessageType: "text", TimestampMS: 1700000000000, Text: "Incoming message", Data: json.RawMessage(`{"type":"text","text":{"body":"Incoming message"}}`)}
}

func cloudMarkerCount(t *testing.T, inbox *Inbox) int {
	t.Helper()
	var count int
	if err := inbox.db.QueryRow("SELECT COUNT(*) FROM inbox_cloud_event_markers").Scan(&count); err != nil {
		t.Fatal(err)
	}
	return count
}

func TestCloudDeliveryConcurrentDuplicatesPublishOnce(t *testing.T) {
	inbox := inboxTestOpen(t, 100)
	message, event := cloudStorageMessage("same-message"), cloudStorageEvent("same-message")
	type result struct {
		events []CloudEvent
		err    error
	}
	results := make(chan result, 16)
	var workers sync.WaitGroup
	for range 16 {
		workers.Add(1)
		go func() {
			defer workers.Done()
			// A webhook can also repeat the same item inside one delivery.
			fresh, err := inbox.RecordCloudDelivery([]MessageView{message, message}, nil, []CloudEvent{event, event})
			results <- result{fresh, err}
		}()
	}
	workers.Wait()
	close(results)
	published := 0
	for result := range results {
		if result.err != nil {
			t.Fatal(result.err)
		}
		published += len(result.events)
		for _, got := range result.events {
			if got.MessageID != event.MessageID || string(got.Data) != string(event.Data) {
				t.Fatalf("event payload changed: %+v", got)
			}
		}
	}
	if published != 1 || cloudMarkerCount(t, inbox) != 1 {
		t.Fatalf("duplicate delivery published %d events", published)
	}
	page, err := inbox.ListMessages(message.ChatID, "", 10)
	if err != nil || len(page.Messages) != 1 {
		t.Fatalf("duplicate archive rows: %+v, %v", page, err)
	}
}

func TestCloudDeliveryDuplicateSuppressionSurvivesRestart(t *testing.T) {
	dir := t.TempDir()
	inbox, err := NewInbox(dir, 10)
	if err != nil {
		t.Fatal(err)
	}
	message, event := cloudStorageMessage("replayed"), cloudStorageEvent("replayed")
	if fresh, err := inbox.RecordCloudDelivery([]MessageView{message}, nil, []CloudEvent{event}); err != nil || len(fresh) != 1 {
		t.Fatalf("initial delivery failed: %+v, %v", fresh, err)
	}
	if err = inbox.Close(); err != nil {
		t.Fatal(err)
	}
	inbox, err = NewInbox(dir, 10)
	if err != nil {
		t.Fatal(err)
	}
	defer inbox.Close()
	if fresh, err := inbox.RecordCloudDelivery([]MessageView{message}, nil, []CloudEvent{event}); err != nil || len(fresh) != 0 {
		t.Fatalf("restart replayed an event: %+v, %v", fresh, err)
	}
}

func TestCloudDeliveryFailedTransactionDoesNotClaimEvents(t *testing.T) {
	inbox := inboxTestOpen(t, 10)
	_, err := inbox.db.Exec(`CREATE TRIGGER fail_cloud_marker BEFORE INSERT ON inbox_cloud_event_markers
WHEN NEW.event_key LIKE '%blocked%' BEGIN SELECT RAISE(ABORT, 'test rollback'); END;`)
	if err != nil {
		t.Fatal(err)
	}
	messages := []MessageView{cloudStorageMessage("first"), cloudStorageMessage("blocked")}
	events := []CloudEvent{cloudStorageEvent("first"), cloudStorageEvent("blocked")}
	if fresh, err := inbox.RecordCloudDelivery(messages, nil, events); err == nil || len(fresh) != 0 {
		t.Fatalf("failed transaction exposed events: %+v, %v", fresh, err)
	}
	if cloudMarkerCount(t, inbox) != 0 {
		t.Fatal("failed transaction retained a marker")
	}
	page, err := inbox.ListMessages(messages[0].ChatID, "", 10)
	if err != nil || len(page.Messages) != 0 {
		t.Fatalf("failed transaction retained messages: %+v, %v", page, err)
	}
	if _, err = inbox.db.Exec("DROP TRIGGER fail_cloud_marker"); err != nil {
		t.Fatal(err)
	}
	if fresh, err := inbox.RecordCloudDelivery(messages, nil, events); err != nil || len(fresh) != 2 {
		t.Fatalf("retry lost rolled-back events: %+v, %v", fresh, err)
	}
}

func TestCloudDeliveryStatusOrderDoesNotSuppressDistinctEvents(t *testing.T) {
	inbox := inboxTestOpen(t, 10)
	message := cloudStorageMessage("outgoing")
	message.FromMe, message.SenderID = true, "me"
	submitted := "submitted"
	message.Status = &submitted
	if _, err := inbox.RecordExternal([]MessageView{message}, nil); err != nil {
		t.Fatal(err)
	}
	for n, status := range []string{"read", "sent", "delivered", "failed", "read", "sent", "delivered", "failed"} {
		event := cloudStorageEvent(message.MessageID)
		event.Kind, event.Status = "message.status", status
		receipt := ExternalReceipt{MessageID: message.MessageID, ChatID: message.ChatID, Status: status}
		if status == "sent" {
			timestamp := int64(1699999990000)
			receipt.SentAtMS = &timestamp
		}
		fresh, err := inbox.RecordCloudDelivery(nil, []ExternalReceipt{receipt}, []CloudEvent{event})
		want := 0
		if n < 4 {
			want = 1
		}
		if err != nil || len(fresh) != want {
			t.Fatalf("status %s at step %d produced %d events; want %d: %v", status, n, len(fresh), want, err)
		}
	}
	page, err := inbox.ListMessages(message.ChatID, "", 10)
	if err != nil || len(page.Messages) != 1 || page.Messages[0].Status == nil || *page.Messages[0].Status != "read" || page.Messages[0].TimestampMS != 1699999990000 {
		t.Fatalf("event claims regressed archive status or sent time: %+v, %v", page, err)
	}
}

func TestCloudDeliveryMarkerRetentionAndClear(t *testing.T) {
	inbox := inboxTestOpen(t, 2)
	for n := range 12 {
		id := fmt.Sprintf("message-%02d", n)
		if _, err := inbox.RecordCloudDelivery([]MessageView{cloudStorageMessage(id)}, nil, []CloudEvent{cloudStorageEvent(id)}); err != nil {
			t.Fatal(err)
		}
	}
	if count := cloudMarkerCount(t, inbox); count != 10 {
		t.Fatalf("retained %d markers, want 10", count)
	}
	if err := inbox.SetLimit(1); err != nil {
		t.Fatal(err)
	}
	if count := cloudMarkerCount(t, inbox); count != 5 {
		t.Fatalf("reduced limit retained %d markers, want 5", count)
	}
	latest := cloudStorageEvent("message-11")
	if fresh, err := inbox.RecordCloudDelivery(nil, nil, []CloudEvent{latest}); err != nil || len(fresh) != 0 {
		t.Fatalf("recent marker was evicted: %+v, %v", fresh, err)
	}
	old := cloudStorageEvent("message-00")
	if fresh, err := inbox.RecordCloudDelivery(nil, nil, []CloudEvent{old}); err != nil || len(fresh) != 1 {
		t.Fatalf("old marker was not evicted: %+v, %v", fresh, err)
	}
	if err := inbox.Clear(); err != nil {
		t.Fatal(err)
	}
	if cloudMarkerCount(t, inbox) != 0 {
		t.Fatal("clearing the archive retained event markers")
	}
	if fresh, err := inbox.RecordCloudDelivery(nil, nil, []CloudEvent{latest}); err != nil || len(fresh) != 1 {
		t.Fatalf("cleared marker still suppressed an event: %+v, %v", fresh, err)
	}
}

func TestCloudDeliveryEventKeysIsolateAccountsAndPhones(t *testing.T) {
	inbox := inboxTestOpen(t, 10)
	original := cloudStorageEvent("shared-id")
	account, phone, kind, status, message := original, original, original, original, original
	account.BusinessAccountID = "987654322"
	phone.PhoneNumberID = "123456780"
	kind.Kind = "message.status"
	status.Status = "sent"
	message.MessageID = "different-id"
	events := []CloudEvent{original, account, phone, kind, status, message}
	if fresh, err := inbox.RecordCloudDelivery(nil, nil, events); err != nil || len(fresh) != len(events) {
		t.Fatalf("distinct event keys collided: %+v, %v", fresh, err)
	}
	if fresh, err := inbox.RecordCloudDelivery(nil, nil, events); err != nil || len(fresh) != 0 {
		t.Fatalf("stable keys did not suppress replay: %+v, %v", fresh, err)
	}
}
