package main

import (
	"database/sql"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func inboxTestMessage(id string, stamp int64) MessageView {
	return MessageView{MessageID: id, ChatID: "15551234567@s.whatsapp.net", SenderID: "15551234567@s.whatsapp.net", TimestampMS: stamp, Kind: "text", Text: "A retained message", Source: "live"}
}
func inboxTestOpen(t *testing.T, limit int) *Inbox {
	t.Helper()
	inbox, err := NewInbox(t.TempDir(), limit)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = inbox.Close() })
	return inbox
}
func TestInboxReplayPagingAndPrivateStorage(t *testing.T) {
	dir := t.TempDir()
	inbox, err := NewInbox(dir, 100)
	if err != nil {
		t.Fatal(err)
	}
	for n := int64(1); n <= 5; n++ {
		m := inboxTestMessage(string(rune('a'+n)), n)
		if err = inbox.Record([]MessageView{m, m}); err != nil {
			t.Fatal(err)
		}
	}
	if err = inbox.Close(); err != nil {
		t.Fatal(err)
	}
	inbox, err = NewInbox(dir, 100)
	if err != nil {
		t.Fatal(err)
	}
	defer inbox.Close()
	page, err := inbox.ListMessages("15551234567:2@s.whatsapp.net", "", 2)
	if err != nil {
		t.Fatal(err)
	}
	if len(page.Messages) != 2 || page.Messages[0].TimestampMS != 5 || page.NextCursor == nil {
		t.Fatalf("unexpected first page: %+v", page)
	}
	second, err := inbox.ListMessages("15551234567@s.whatsapp.net", *page.NextCursor, 2)
	if err != nil {
		t.Fatal(err)
	}
	if second.Messages[0].TimestampMS != 3 {
		t.Fatalf("wrong older page: %+v", second)
	}
	if _, err = inbox.ListMessages("99999999@s.whatsapp.net", *page.NextCursor, 2); err == nil {
		t.Fatal("cross-chat cursor accepted")
	}
	if _, err = inbox.ListMessages("15551234567@s.whatsapp.net", "invalid!", 2); err == nil {
		t.Fatal("malformed cursor accepted")
	}
	chats, err := inbox.ListChats("retained", "", 100)
	if err != nil || len(chats.Chats) != 1 || chats.Chats[0].MessageCount != 5 {
		t.Fatalf("unexpected search result: %+v, %v", chats, err)
	}
	for _, p := range []string{dir, filepath.Join(dir, "inbox.db")} {
		info, err := os.Stat(p)
		if err != nil {
			t.Fatal(err)
		}
		if info.Mode().Perm()&0077 != 0 {
			t.Fatalf("private data has permissive mode: %v", info.Mode())
		}
	}
}
func TestInboxRetentionExpirationAndNoBodyResurrection(t *testing.T) {
	inbox := inboxTestOpen(t, 3)
	expiry := time.Now().Add(-time.Hour).UnixMilli()
	expired := inboxTestMessage("expired", 1)
	expired.ExpiresAtMS = &expiry
	viewOnce := inboxTestMessage("view-once", 2)
	viewOnce.Kind = "view_once"
	deleted := inboxTestMessage("deleted", 3)
	deleted.Kind = "deleted"
	if err := inbox.Record([]MessageView{expired, viewOnce, deleted}); err != nil {
		t.Fatal(err)
	}
	for _, id := range []string{"expired", "view-once", "deleted"} {
		if err := inbox.Record([]MessageView{inboxTestMessage(id, 4)}); err != nil {
			t.Fatal(err)
		}
	}
	page, err := inbox.ListMessages(expired.ChatID, "", 10)
	if err != nil {
		t.Fatal(err)
	}
	for _, m := range page.Messages {
		if m.Text != "" || m.Kind == "text" {
			t.Fatalf("protected body restored: %+v", m)
		}
	}
	if err = inbox.SetLimit(1); err != nil {
		t.Fatal(err)
	}
	page, err = inbox.ListMessages(expired.ChatID, "", 10)
	if err != nil || len(page.Messages) != 1 || !page.Coverage.RetentionTruncated {
		t.Fatalf("retention did not apply: %+v, %v", page, err)
	}
}
func TestInboxEditsAndGroupSenderIdentity(t *testing.T) {
	inbox := inboxTestOpen(t, 10)
	first := inboxTestMessage("same", 1)
	first.ChatID = "123-456@g.us"
	second := first
	second.SenderID = "15559876543@s.whatsapp.net"
	second.Text = "Second sender"
	if err := inbox.Record([]MessageView{first, second}); err != nil {
		t.Fatal(err)
	}
	edit := first
	edit.Edited = true
	edit.Text = "Edited"
	if err := inbox.Record([]MessageView{edit, first}); err != nil {
		t.Fatal(err)
	}
	page, err := inbox.ListMessages(first.ChatID, "", 10)
	if err != nil || len(page.Messages) != 2 {
		t.Fatalf("sender collision: %+v %v", page, err)
	}
	for _, m := range page.Messages {
		if m.SenderID == first.SenderID && m.Text != "Edited" {
			t.Fatal("stale replay replaced edit")
		}
	}
}
func TestInboxGenerationAccountAndHistory(t *testing.T) {
	inbox := inboxTestOpen(t, 10)
	if err := inbox.SetAccount("account-a"); err != nil {
		t.Fatal(err)
	}
	capture := inbox.Capture()
	progress := 55
	m := inboxTestMessage("history", 1)
	m.Source = "history"
	if err := capture.RecordHistory([]MessageView{m}, map[string]string{m.ChatID: "Archived contact"}, &progress); err != nil {
		t.Fatal(err)
	}
	page, err := inbox.ListChats("", "", 10)
	if err != nil || page.Coverage.LastHistorySyncProgress == nil || *page.Coverage.LastHistorySyncProgress != 55 {
		t.Fatalf("history coverage: %+v, %v", page, err)
	}
	if err = inbox.SetAccount("account-b"); err != nil {
		t.Fatal(err)
	}
	if err = capture.Record([]MessageView{m}); err == nil {
		t.Fatal("obsolete capture was accepted")
	}
	if err = capture.RecordReceipt(m.ChatID, []string{m.MessageID}, "read"); err == nil {
		t.Fatal("obsolete receipt capture was accepted")
	}
	page, err = inbox.ListChats("", "", 10)
	if err != nil || len(page.Chats) != 0 {
		t.Fatal("old account rows remained")
	}
	capture = inbox.Capture()
	if err = inbox.Clear(); err != nil {
		t.Fatal(err)
	}
	if err = capture.Record([]MessageView{m}); err == nil {
		t.Fatal("cleared capture was accepted")
	}
	if _, err = inbox.Oldest(m.ChatID); err == nil {
		t.Fatal("empty inbox has history anchor")
	}
}
func TestInboxEarlyCloudReceiptsArePersistentMonotonicAndIsolated(t *testing.T) {
	dir := t.TempDir()
	inbox, err := NewInbox(dir, 10)
	if err != nil {
		t.Fatal(err)
	}
	receipt := ExternalReceipt{MessageID: "outgoing", ChatID: "cloud:BR.ENT.private123", Status: "read"}
	if _, err = inbox.RecordExternal(nil, []ExternalReceipt{receipt}); err != nil {
		t.Fatal(err)
	}
	_ = inbox.Close()
	inbox, err = NewInbox(dir, 10)
	if err != nil {
		t.Fatal(err)
	}
	defer inbox.Close()
	m := inboxTestMessage("outgoing", 1)
	m.ChatID = "cloud:+15551234567"
	m.FromMe = true
	status := "submitted"
	m.Status = &status
	if _, err = inbox.RecordExternal([]MessageView{m, m}, nil); err != nil {
		t.Fatal(err)
	}
	receipt.Status = "delivered"
	if _, err = inbox.RecordExternal(nil, []ExternalReceipt{receipt}); err != nil {
		t.Fatal(err)
	}
	page, err := inbox.ListMessages(m.ChatID, "", 10)
	if err != nil || len(page.Messages) != 1 || page.Messages[0].Status == nil || *page.Messages[0].Status != "read" {
		t.Fatalf("receipt lost: %+v, %v", page, err)
	}
	native, err := inbox.ListMessages("15551234567@s.whatsapp.net", "", 10)
	if err != nil || len(native.Messages) != 0 {
		t.Fatal("cloud rows leaked into native chat")
	}
	bad := m
	bad.ChatID = "../outside"
	if _, err = inbox.RecordExternal([]MessageView{inboxTestMessage("invalid-native", 2), bad}, nil); err == nil {
		t.Fatal("invalid external batch accepted")
	}
	if err = inbox.Clear(); err != nil {
		t.Fatal(err)
	}
	if _, err = inbox.RecordExternal([]MessageView{m}, nil); err != nil {
		t.Fatal(err)
	}
	page, _ = inbox.ListMessages(m.ChatID, "", 10)
	if *page.Messages[0].Status != "submitted" {
		t.Fatal("clear retained old receipts")
	}
}

func TestInboxUpgradesLegacyReceiptsAndPreservesEarlySentTime(t *testing.T) {
	dir := t.TempDir()
	legacy, err := sql.Open("sqlite", filepath.Join(dir, "inbox.db"))
	if err != nil {
		t.Fatal(err)
	}
	_, err = legacy.Exec(`CREATE TABLE inbox_external_receipts(message_id TEXT PRIMARY KEY,chat_id TEXT NOT NULL,status TEXT NOT NULL,rank INTEGER NOT NULL,received_at_ms BIGINT NOT NULL);
INSERT INTO inbox_external_receipts VALUES('outgoing','cloud:+15551234567','read',4,1700000005000);`)
	if closeErr := legacy.Close(); err == nil {
		err = closeErr
	}
	if err != nil {
		t.Fatal(err)
	}
	inbox, err := NewInbox(dir, 10)
	if err != nil {
		t.Fatal(err)
	}
	sentAt := int64(1700000000000)
	_, err = inbox.RecordExternal(nil, []ExternalReceipt{{MessageID: "outgoing", ChatID: "cloud:+15551234567", Status: "sent", SentAtMS: &sentAt}})
	if closeErr := inbox.Close(); err == nil {
		err = closeErr
	}
	if err != nil {
		t.Fatal(err)
	}
	// A callback can be committed before the send response is archived, even across a restart.
	inbox, err = NewInbox(dir, 10)
	if err != nil {
		t.Fatal(err)
	}
	defer inbox.Close()
	message := inboxTestMessage("outgoing", 1700000018000)
	message.ChatID = "cloud:+15551234567"
	message.FromMe = true
	submitted := "submitted"
	message.Status = &submitted
	if _, err = inbox.RecordExternal([]MessageView{message}, nil); err != nil {
		t.Fatal(err)
	}
	page, err := inbox.ListMessages(message.ChatID, "", 10)
	if err != nil || len(page.Messages) != 1 {
		t.Fatalf("message missing after upgrade: %+v, %v", page, err)
	}
	got := page.Messages[0]
	if got.TimestampMS != sentAt || got.Status == nil || *got.Status != "read" {
		t.Fatalf("upgrade or early receipt lost: %+v", got)
	}
}

func TestInboxMessageSequenceMatchesPaginationAndSurvivesReplay(t *testing.T) {
	inbox := inboxTestOpen(t, 10)
	first := inboxTestMessage("z-first", 1700000000000)
	second := inboxTestMessage("a-second", first.TimestampMS)
	if err := inbox.Record([]MessageView{first, second}); err != nil {
		t.Fatal(err)
	}
	newest, err := inbox.ListMessages(first.ChatID, "", 1)
	if err != nil || len(newest.Messages) != 1 || newest.NextCursor == nil || newest.Messages[0].MessageID != second.MessageID {
		t.Fatalf("bad first page: %+v, %v", newest, err)
	}
	older, err := inbox.ListMessages(first.ChatID, *newest.NextCursor, 1)
	if err != nil || len(older.Messages) != 1 || older.Messages[0].MessageID != first.MessageID {
		t.Fatalf("bad older page: %+v, %v", older, err)
	}
	sequence := func(message MessageView) int64 {
		t.Helper()
		raw, _ := json.Marshal(message)
		var projection struct {
			Sequence int64 `json:"sequence"`
		}
		if err := json.Unmarshal(raw, &projection); err != nil {
			t.Fatal(err)
		}
		return projection.Sequence
	}
	newestSequence, olderSequence := sequence(newest.Messages[0]), sequence(older.Messages[0])
	if olderSequence <= 0 || newestSequence <= olderSequence {
		t.Fatalf("API must expose its chronological tie order: first=%d, second=%d", olderSequence, newestSequence)
	}
	if err := inbox.Record([]MessageView{first, second}); err != nil {
		t.Fatal(err)
	}
	page, err := inbox.ListMessages(first.ChatID, "", 10)
	if err != nil || len(page.Messages) != 2 || sequence(page.Messages[0]) != newestSequence || sequence(page.Messages[1]) != olderSequence {
		t.Fatalf("replay changed the message order: %+v, %v", page, err)
	}
}

func TestInboxExternalReceiptsOnlyCorrectOutgoingSentTimes(t *testing.T) {
	inbox := inboxTestOpen(t, 10)
	outgoing := inboxTestMessage("shared-id", 1700000018000)
	outgoing.ChatID = "cloud:+15551234567"
	outgoing.FromMe = true
	incoming := outgoing
	incoming.FromMe = false
	incoming.TimestampMS = 1700000005000
	if _, err := inbox.RecordExternal([]MessageView{outgoing, incoming}, nil); err != nil {
		t.Fatal(err)
	}
	for _, state := range []string{"delivered", "read"} {
		if _, err := inbox.RecordExternal(nil, []ExternalReceipt{{MessageID: outgoing.MessageID, ChatID: outgoing.ChatID, Status: state}}); err != nil {
			t.Fatal(err)
		}
		page, err := inbox.ListMessages(outgoing.ChatID, "", 10)
		if err != nil || len(page.Messages) != 2 || page.Messages[0].TimestampMS != outgoing.TimestampMS {
			t.Fatalf("%s changed the provisional send time: %+v, %v", state, page, err)
		}
	}
	sentAt := int64(1700000000000)
	if _, err := inbox.RecordExternal(nil, []ExternalReceipt{{MessageID: outgoing.MessageID, ChatID: outgoing.ChatID, Status: "sent", SentAtMS: &sentAt}}); err != nil {
		t.Fatal(err)
	}
	page, err := inbox.ListMessages(outgoing.ChatID, "", 10)
	if err != nil || len(page.Messages) != 2 || page.Messages[0].FromMe || page.Messages[0].TimestampMS != incoming.TimestampMS || !page.Messages[1].FromMe || page.Messages[1].TimestampMS != sentAt {
		t.Fatalf("sent receipt changed the wrong identity or failed to restore chronology: %+v, %v", page, err)
	}
	invalid := outgoing
	invalid.MessageID = "rejected-batch"
	negative := int64(-1)
	if _, err := inbox.RecordExternal([]MessageView{invalid}, []ExternalReceipt{{MessageID: invalid.MessageID, ChatID: invalid.ChatID, Status: "sent", SentAtMS: &negative}}); err == nil {
		t.Fatal("negative sent time accepted")
	}
	page, err = inbox.ListMessages(outgoing.ChatID, "", 10)
	if err != nil || len(page.Messages) != 2 {
		t.Fatalf("invalid receipt partially committed its batch: %+v, %v", page, err)
	}
}
func TestInboxRejectsSymlinksAndSerializesOnlyDisplayFields(t *testing.T) {
	dir := t.TempDir()
	target := filepath.Join(t.TempDir(), "target")
	if err := os.WriteFile(target, []byte("untouched"), 0600); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(target, filepath.Join(dir, "inbox.db")); err != nil {
		t.Fatal(err)
	}
	if _, err := NewInbox(dir, 10); err == nil {
		t.Fatal("symlink DB was opened")
	}
	msg := inboxTestMessage("test", 1)
	expiry := int64(42)
	msg.ExpiresAtMS = &expiry
	msg.Edited = true
	raw, _ := json.Marshal(msg)
	if strings.Contains(string(raw), "expires") || strings.Contains(string(raw), "edited") {
		t.Fatalf("internal fields exposed: %s", raw)
	}
}
