package main

import (
	"testing"
	"time"
)

func TestCloudSendRetainsRequestTimeUntilConfirmation(t *testing.T) {
	inbox, err := NewInbox(t.TempDir(), 10)
	if err != nil {
		t.Fatal(err)
	}
	defer inbox.Close()
	requestedAt := time.Unix(1700000000, 123000000)
	// A reply can be captured before the HTTP send response is archived.
	reply := MessageView{MessageID: "reply", ChatID: "cloud:+15551234567", SenderID: "+15551234567", TimestampMS: requestedAt.Add(11 * time.Second).UnixMilli(), Kind: "text", Text: "Reply"}
	if _, err = inbox.RecordExternal([]MessageView{reply}, nil); err != nil {
		t.Fatal(err)
	}
	sent := CloudSentMessage{MessageID: "outgoing", ChatID: "+15551234567"}
	if err = retainCloudSend(inbox, sent, "text", "Earlier send", requestedAt); err != nil {
		t.Fatal(err)
	}
	page, err := inbox.ListMessages(reply.ChatID, "", 10)
	if err != nil {
		t.Fatal(err)
	}
	if len(page.Messages) != 2 || page.Messages[0].MessageID != "reply" || page.Messages[1].TimestampMS != requestedAt.UnixMilli() {
		t.Fatalf("send completion moved the message after its reply: %+v", page.Messages)
	}
}
