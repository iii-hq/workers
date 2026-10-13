package main

import (
	"context"
	"encoding/json"
	"fmt"
	"math"
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"go.mau.fi/whatsmeow/proto/waCommon"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/proto/waHistorySync"
	"go.mau.fi/whatsmeow/proto/waWeb"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
)

func testNativeMessage(text string) *events.Message {
	chat := types.NewJID("15551234567", types.DefaultUserServer)
	return &events.Message{Info: types.MessageInfo{MessageSource: types.MessageSource{Chat: chat, Sender: chat}, ID: "test-message", Timestamp: time.Now()}, Message: &waE2E.Message{Conversation: &text}}
}

func TestNativeViewOnceNeverProjectsContent(t *testing.T) {
	for _, msg := range []*waE2E.Message{
		{ImageMessage: &waE2E.ImageMessage{Caption: proto.String("private caption"), ViewOnce: proto.Bool(true)}},
		{VideoMessage: &waE2E.VideoMessage{Caption: proto.String("private caption"), ViewOnce: proto.Bool(true)}},
		{AudioMessage: &waE2E.AudioMessage{ViewOnce: proto.Bool(true)}},
		{ExtendedTextMessage: &waE2E.ExtendedTextMessage{Text: proto.String("private text"), ViewOnce: proto.Bool(true)}},
	} {
		event := testNativeMessage("")
		event.Message = msg
		view := nativeMessageView(event, "live")
		if view.Kind != "view_once" || view.Text != "" {
			t.Fatalf("view-once content retained: %#v", view)
		}
	}
	event := testNativeMessage("private text")
	event.IsViewOnceV2 = true
	if view := nativeMessageView(event, "live"); view.Text != "" || view.Kind != "view_once" {
		t.Fatal("view-once wrapper ignored")
	}
	event.IsViewOnceV2 = false
	event.Message = &waE2E.Message{ProtocolMessage: &waE2E.ProtocolMessage{Type: waE2E.ProtocolMessage_MESSAGE_EDIT.Enum(), Key: &waCommon.MessageKey{ID: proto.String("original")}, EditedMessage: &waE2E.Message{ImageMessage: &waE2E.ImageMessage{Caption: proto.String("private caption"), ViewOnce: proto.Bool(true)}}}}
	if view := nativeMessageView(event, "live"); view.Text != "" || view.Kind != "view_once" || view.MessageID != "original" {
		t.Fatal("edited view-once payload retained content or lost target")
	}
}

func TestNativeEphemeralExpiryUsesMessageTimeAndHistoryBounds(t *testing.T) {
	now := time.Now().Truncate(time.Second)
	event := testNativeMessage("")
	event.Info.Timestamp = now
	event.Message = &waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{Text: proto.String("temporary"), ContextInfo: &waE2E.ContextInfo{Expiration: proto.Uint32(60), EphemeralSettingTimestamp: proto.Int64(now.Add(24 * time.Hour).Unix())}}}
	view := nativeMessageView(event, "live")
	if view.ExpiresAtMS == nil || *view.ExpiresAtMS != now.Add(time.Minute).UnixMilli() {
		t.Fatalf("future setting timestamp extended retention: %#v", view)
	}
	event = testNativeMessage("history secret")
	event.SourceWebMsg = &waWeb.WebMessageInfo{EphemeralStartTimestamp: proto.Uint64(uint64(now.Add(-2 * time.Hour).Unix())), EphemeralDuration: proto.Uint32(3600)}
	view = nativeMessageView(event, "history")
	if view.Kind != "expired" || view.Text != "" || view.ExpiresAtMS == nil {
		t.Fatalf("history expiration ignored: %#v", view)
	}
	event.SourceWebMsg = &waWeb.WebMessageInfo{EphemeralExpirationTimestamp: proto.Uint64(uint64(now.Add(-time.Second).Unix()))}
	view = nativeMessageView(event, "history")
	if view.Kind != "expired" || view.Text != "" {
		t.Fatalf("absolute expiration ignored: %#v", view)
	}
	for _, web := range []*waWeb.WebMessageInfo{{EphemeralStartTimestamp: proto.Uint64(uint64(now.Unix()))}, {EphemeralStartTimestamp: proto.Uint64(math.MaxUint64), EphemeralDuration: proto.Uint32(3600)}, {EphemeralExpirationTimestamp: proto.Uint64(math.MaxUint64)}} {
		event.SourceWebMsg = web
		view = nativeMessageView(event, "history")
		if view.Kind != "ephemeral" || view.Text != "" {
			t.Fatalf("unknown expiry retained content: %#v", view)
		}
	}
}

func TestNativeHistoryEditRecoversTargetAndEditState(t *testing.T) {
	event := testNativeMessage("edited text")
	event.Info.ID = "envelope"
	event.SourceWebMsg = &waWeb.WebMessageInfo{Message: &waE2E.Message{ProtocolMessage: &waE2E.ProtocolMessage{Type: waE2E.ProtocolMessage_MESSAGE_EDIT.Enum(), Key: &waCommon.MessageKey{ID: proto.String("original"), FromMe: proto.Bool(false), Participant: proto.String("998877@lid")}, EditedMessage: &waE2E.Message{Conversation: proto.String("edited text")}}}}
	view := nativeMessageView(event, "history")
	if !view.Edited || view.MessageID != "original" || view.SenderID != "998877@lid" || view.Text != "edited text" {
		t.Fatalf("history edit lost target: %#v", view)
	}
}

func TestLateCallbacksCannotChangeReplacementClient(t *testing.T) {
	m, s := testSessionManager(t)
	old := testOfflineNativeClient(t, s)
	oldCapture := s.inbox.Capture()
	if _, err := m.Stop(context.Background(), "test", true); err != nil {
		t.Fatal(err)
	}
	current := testOfflineNativeClient(t, s)
	capture := s.inbox.Capture()
	if !m.handleNativeEvent(s, current, capture, testNativeMessage("new account")) {
		t.Fatal("current message failed")
	}
	before := s.nextSequence
	for _, event := range []any{&events.LoggedOut{}, &events.PairSuccess{ID: types.NewJID("19999999999", types.DefaultUserServer)}, &events.Connected{}, &events.Disconnected{}, testNativeMessage("old account")} {
		if !m.handleNativeEvent(s, old, oldCapture, event) {
			t.Fatal("late callback was not ignored")
		}
	}
	m.connectionFailed(s, old)
	if s.status != "starting" || s.nextSequence != before {
		t.Fatal("stale callback changed current status or emitted event")
	}
	page, err := s.inbox.ListMessages("15551234567@s.whatsapp.net", "", 10)
	if err != nil || len(page.Messages) != 1 || page.Messages[0].Text != "new account" {
		t.Fatalf("late callback changed archive: %#v %v", page, err)
	}
}

func TestRemoteLogoutInvalidatesQueuedEventsAndLateCallbacks(t *testing.T) {
	m, s := testSessionManager(t)
	client := testOfflineNativeClient(t, s)
	capture := s.inbox.Capture()
	if !m.handleNativeEvent(s, client, capture, testNativeMessage("old account")) {
		t.Fatal("message failed")
	}
	queued := <-m.EventQueue
	if !m.eventCurrent(queued) {
		t.Fatal("active event rejected")
	}
	if !m.handleNativeEvent(s, client, capture, &events.LoggedOut{}) {
		t.Fatal("logout failed")
	}
	if m.eventCurrent(queued) {
		t.Fatal("queued private event survived logout")
	}
	if !m.handleNativeEvent(s, client, capture, &events.Connected{}) {
		t.Fatal("late connected callback failed")
	}
	if !m.handleNativeEvent(s, client, capture, testNativeMessage("late old content")) {
		t.Fatal("late message failed")
	}
	if s.status != "logged_out" {
		t.Fatal("late callback revived logged-out session")
	}
	page, err := s.inbox.ListChats("", "", 10)
	if err != nil || len(page.Chats) != 0 {
		t.Fatalf("archive repopulated after logout: %#v %v", page, err)
	}
	if err := capture.Record([]MessageView{nativeMessageView(testNativeMessage("stale"), "live")}); err == nil {
		t.Fatal("stale capture remained valid")
	}
	polled, err := m.Poll("test", 0, 10)
	if err != nil || len(polled.Events) != 1 || polled.Events[0].Kind != "session.logged_out" {
		t.Fatalf("logout retained message stream: %#v %v", polled, err)
	}
}

func TestAccountChangeRefreshesCaptureAndInvalidatesOldEvents(t *testing.T) {
	m, s := testSessionManager(t)
	client := testOfflineNativeClient(t, s)
	if err := s.inbox.SetAccount("15550000001@s.whatsapp.net"); err != nil {
		t.Fatal(err)
	}
	capture := s.inbox.Capture()
	if !m.handleNativeEvent(s, client, capture, testNativeMessage("old account")) {
		t.Fatal("old message failed")
	}
	queued := <-m.EventQueue
	if !m.handleNativeEvent(s, client, capture, &events.PairSuccess{ID: types.NewJID("15550000002", types.DefaultUserServer)}) {
		t.Fatal("account refresh failed")
	}
	if m.eventCurrent(queued) {
		t.Fatal("old account event survived identity change")
	}
	if !m.handleNativeEvent(s, client, capture, testNativeMessage("new account")) {
		t.Fatal("new account capture remained invalid")
	}
	page, err := s.inbox.ListMessages("15551234567@s.whatsapp.net", "", 10)
	if err != nil || len(page.Messages) != 1 || page.Messages[0].Text != "new account" {
		t.Fatalf("account archive not isolated: %#v %v", page, err)
	}
}

func TestHistoryCaptureBatchesAndSkipsUnsupportedChats(t *testing.T) {
	m, s := testSessionManager(t)
	client := testOfflineNativeClient(t, s)
	cfg := m.Config()
	cfg.MaxInboxMessages = 100
	if _, err := m.ApplyConfig(cfg); err != nil {
		t.Fatal(err)
	}
	messages := make([]*waHistorySync.HistorySyncMsg, 10001)
	for n := range messages {
		messages[n] = &waHistorySync.HistorySyncMsg{Message: &waWeb.WebMessageInfo{Key: &waCommon.MessageKey{ID: proto.String(fmt.Sprintf("m-%05d", n)), FromMe: proto.Bool(false)}, MessageTimestamp: proto.Uint64(uint64(time.Now().Unix())), Message: &waE2E.Message{Conversation: proto.String("history")}}}
	}
	event := &events.HistorySync{Data: &waHistorySync.HistorySync{Progress: proto.Uint32(80), Conversations: []*waHistorySync.Conversation{{ID: proto.String("status@broadcast"), Name: proto.String("not an inbox")}, {ID: proto.String("15551234567@s.whatsapp.net"), Name: proto.String("Test contact"), Messages: messages}}}}
	count, err := captureNativeHistory(client, s.inbox.Capture(), event)
	if err != nil || count != len(messages) {
		t.Fatalf("history capture: %d %v", count, err)
	}
	page, err := s.inbox.ListChats("", "", 10)
	if err != nil || len(page.Chats) != 1 || page.Chats[0].MessageCount != 100 || page.Chats[0].Title != "Test contact" || !page.Coverage.RetentionTruncated || page.Coverage.LastHistorySyncProgress == nil || *page.Coverage.LastHistorySyncProgress != 80 {
		t.Fatalf("history coverage/retention: %#v %v", page, err)
	}
}

func TestEventsBoundedSequenceAndRestartReservation(t *testing.T) {
	m, s := testSessionManager(t)
	// MaxEvents is structural, so use the startup setting for this in-memory stream.
	m.mu.Lock()
	m.config.MaxEvents = 2
	m.mu.Unlock()
	for n := 0; n < 3; n++ {
		if err := m.emit(s, "test", map[string]any{"n": n}); err != nil {
			t.Fatal(err)
		}
	}
	page, err := m.Poll("test", 0, 10)
	if err != nil || !page.Truncated || len(page.Events) != 2 || page.NextSequence != 3 {
		t.Fatalf("bounded event stream: %#v %v", page, err)
	}
	page, err = m.Poll("test", math.MaxUint64, 10)
	if err != nil || page.Truncated || len(page.Events) != 0 || page.NextSequence != math.MaxUint64 {
		t.Fatalf("overflowing cursor: %#v %v", page, err)
	}
	cfg := m.Config()
	if err = m.Shutdown(context.Background()); err != nil {
		t.Fatal(err)
	}
	reopened, err := newTestManager(cfg)
	if err != nil {
		t.Fatal(err)
	}
	defer reopened.Shutdown(context.Background())
	current, _ := reopened.session("test")
	if err = reopened.emit(current, "test", nil); err != nil {
		t.Fatal(err)
	}
	page, err = reopened.Poll("test", 3, 10)
	if err != nil || len(page.Events) != 1 || page.Events[0].Sequence <= 3 || !page.Truncated {
		t.Fatalf("restart reused cursor: %#v %v", page, err)
	}
}

func TestEventRouterRejectsTrailingOrUnknownFilterData(t *testing.T) {
	router := &EventRouter{}
	for _, raw := range []string{`{"session_id":"test"} {}`, `{"secret":"ignored"}`, `{"kinds":["bad kind"]}`, `{"session_id":"../outside"}`} {
		if err := router.RegisterTrigger(context.Background(), iii.TriggerConfig{ID: "test", Config: json.RawMessage(raw)}); err == nil {
			t.Errorf("invalid filter accepted: %s", raw)
		}
	}
	if err := router.RegisterTrigger(context.Background(), iii.TriggerConfig{ID: "test", Config: json.RawMessage(`{"session_id":"test","kinds":["message.received"]}`)}); err != nil {
		t.Fatal(err)
	}
}
