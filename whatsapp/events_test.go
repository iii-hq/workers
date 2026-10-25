package main

import (
	"context"
	"encoding/json"
	"fmt"
	"math"
	"slices"
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

func TestNativeMessageEventContext(t *testing.T) {
	type quoted struct {
		ID          string `json:"id"`
		Participant string `json:"participant"`
		IsMe        bool   `json:"is_me"`
		Kind        string `json:"kind"`
		Text        string `json:"text"`
		FileName    string `json:"file_name"`
	}
	type contextEvent struct {
		Info struct {
			ID          string `json:"id"`
			TimestampMS int64  `json:"timestamp_ms"`
			PushName    string `json:"push_name"`
			Edited      bool   `json:"edited"`
			Source      struct {
				Chat           string  `json:"chat"`
				Sender         string  `json:"sender"`
				SenderAlt      *string `json:"sender_alt"`
				IsFromMe       bool    `json:"is_from_me"`
				IsGroup        bool    `json:"is_group"`
				AddressingMode *string `json:"addressing_mode"`
			} `json:"source"`
		} `json:"info"`
		Message struct {
			Conversation    string   `json:"conversation"`
			MentionedJIDs   []string `json:"mentioned_jids"`
			MentionsMe      bool     `json:"mentions_me"`
			MentionedMeJIDs []string `json:"mentioned_me_jids"`
			FileName        string   `json:"file_name"`
			Quoted          *quoted  `json:"quoted"`
		} `json:"message"`
		Kind string `json:"kind"`
	}
	own := types.NewJID("5511999990000", types.DefaultUserServer)
	ownLID := types.NewJID("123456789", types.HiddenUserServer)
	question := "@123456789 qual o prazo da ação 3?"
	caption := "*Laudo técnico — Padaria Sol*\nVisita LT-2026-1008-001, realizada em 08/10/2026."
	report := &waE2E.Message{DocumentMessage: &waE2E.DocumentMessage{Caption: proto.String(caption), FileName: proto.String("LT-2026-1008-001.pdf")}}
	for _, tc := range []struct {
		name         string
		context      *waE2E.ContextInfo
		conversation string
		mentioned    []string
		mentionsMe   bool
		mentionedMe  []string
		quoted       *quoted
	}{
		{"mention of our LID and quote of our PDF", &waE2E.ContextInfo{MentionedJID: []string{"123456789@lid"}, StanzaID: proto.String("3EB0ABC"), Participant: proto.String("123456789@lid"), QuotedMessage: report}, question, []string{"123456789@lid"}, true, []string{"123456789@lid"}, &quoted{"3EB0ABC", "123456789@lid", true, "document", caption, "LT-2026-1008-001.pdf"}},
		{"mention of our phone with a device suffix", &waE2E.ContextInfo{MentionedJID: []string{"5511999990000:12@s.whatsapp.net"}}, question, []string{"5511999990000:12@s.whatsapp.net"}, true, []string{"5511999990000:12@s.whatsapp.net"}, nil},
		{"mention of our LID and someone else", &waE2E.ContextInfo{MentionedJID: []string{"5511977776666@s.whatsapp.net", "123456789@lid"}}, question, []string{"5511977776666@s.whatsapp.net", "123456789@lid"}, true, []string{"123456789@lid"}, nil},
		{"mention of someone else", &waE2E.ContextInfo{MentionedJID: []string{"5511977776666@s.whatsapp.net"}}, question, []string{"5511977776666@s.whatsapp.net"}, false, []string{}, nil},
		{"no context", nil, question, []string{}, false, []string{}, nil},
		{"disappearing message", &waE2E.ContextInfo{Expiration: proto.Uint32(86400), MentionedJID: []string{"123456789@lid"}, StanzaID: proto.String("3EB0ABC"), Participant: proto.String("123456789@lid"), QuotedMessage: report}, "", []string{}, false, []string{}, nil},
		{"quote of a disappearing message", &waE2E.ContextInfo{StanzaID: proto.String("3EB0OLD"), Participant: proto.String("5511977776666@s.whatsapp.net"), QuotedMessage: &waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{Text: proto.String("temporary"), ContextInfo: &waE2E.ContextInfo{Expiration: proto.Uint32(86400)}}}}, question, []string{}, false, []string{}, &quoted{"3EB0OLD", "5511977776666@s.whatsapp.net", false, "text", "", ""}},
		{"quote of a disappearing document", &waE2E.ContextInfo{StanzaID: proto.String("3EB0OLD"), Participant: proto.String("5511977776666@s.whatsapp.net"), QuotedMessage: &waE2E.Message{DocumentMessage: &waE2E.DocumentMessage{FileName: proto.String("private.pdf"), ContextInfo: &waE2E.ContextInfo{Expiration: proto.Uint32(86400)}}}}, question, []string{}, false, []string{}, &quoted{"3EB0OLD", "5511977776666@s.whatsapp.net", false, "document", "", ""}},
		{"quote of a captioned document sent from a phone", &waE2E.ContextInfo{StanzaID: proto.String("3EB0DOC"), Participant: proto.String("5511977776666@s.whatsapp.net"), QuotedMessage: &waE2E.Message{DocumentWithCaptionMessage: &waE2E.FutureProofMessage{Message: report}}}, question, []string{}, false, []string{}, &quoted{"3EB0DOC", "5511977776666@s.whatsapp.net", false, "document", caption, "LT-2026-1008-001.pdf"}},
		{"quote of a view-once image", &waE2E.ContextInfo{StanzaID: proto.String("3EB0ONCE"), Participant: proto.String("5511977776666@s.whatsapp.net"), QuotedMessage: &waE2E.Message{ViewOnceMessageV2: &waE2E.FutureProofMessage{Message: &waE2E.Message{ImageMessage: &waE2E.ImageMessage{Caption: proto.String("private caption")}}}}}, question, []string{}, false, []string{}, &quoted{"3EB0ONCE", "5511977776666@s.whatsapp.net", false, "view_once", "", ""}},
		{"quote that carries no message", &waE2E.ContextInfo{StanzaID: proto.String("3EB0GONE"), Participant: proto.String("5511977776666@s.whatsapp.net")}, question, []string{}, false, []string{}, &quoted{"3EB0GONE", "5511977776666@s.whatsapp.net", false, "unknown", "", ""}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			var got contextEvent
			e := &events.Message{Info: types.MessageInfo{MessageSource: types.MessageSource{Chat: types.NewJID("120363000000000001", types.GroupServer), Sender: types.NewJID("987654321", types.HiddenUserServer), IsGroup: true, AddressingMode: types.AddressingModeLID, SenderAlt: types.JID{User: "5511988887777", Device: 7, Server: types.DefaultUserServer}}, ID: "3EB0QUESTION", PushName: "Ana", Timestamp: time.Now()}, Message: &waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{Text: proto.String(question), ContextInfo: tc.context}}}
			raw, err := json.Marshal(nativeMessageEvent(e, nativeMessageView(e, "live"), own, ownLID))
			if err != nil {
				t.Fatal(err)
			}
			if err := json.Unmarshal(raw, &got); err != nil {
				t.Fatal(err)
			}
			info, src, msg := got.Info, got.Info.Source, got.Message
			if info.ID != "3EB0QUESTION" || info.TimestampMS != e.Info.Timestamp.UnixMilli() || info.PushName != "Ana" || info.Edited || got.Kind != "text" {
				t.Fatalf("info: %s", raw)
			}
			if src.Chat != "120363000000000001@g.us" || src.Sender != "987654321@lid" || src.SenderAlt == nil || *src.SenderAlt != "5511988887777@s.whatsapp.net" || src.IsFromMe || !src.IsGroup || src.AddressingMode == nil || *src.AddressingMode != "lid" {
				t.Fatalf("source: %s", raw)
			}
			if msg.Conversation != tc.conversation || msg.MentionedJIDs == nil || !slices.Equal(msg.MentionedJIDs, tc.mentioned) || msg.MentionsMe != tc.mentionsMe || msg.MentionedMeJIDs == nil || !slices.Equal(msg.MentionedMeJIDs, tc.mentionedMe) || msg.FileName != "" {
				t.Fatalf("message: %s", raw)
			}
			if (msg.Quoted == nil) != (tc.quoted == nil) || (msg.Quoted != nil && *msg.Quoted != *tc.quoted) {
				t.Fatalf("quoted: %s", raw)
			}
		})
	}
	var got contextEvent
	e := testNativeMessage("direct")
	raw, _ := json.Marshal(nativeMessageEvent(e, nativeMessageView(e, "live"), types.EmptyJID, types.EmptyJID))
	if err := json.Unmarshal(raw, &got); err != nil || got.Info.Source.SenderAlt != nil || got.Info.Source.AddressingMode != nil || got.Message.Quoted != nil {
		t.Fatalf("empty source fields not null: %s", raw)
	}
	doc := testNativeMessage("")
	doc.Message = &waE2E.Message{DocumentMessage: &waE2E.DocumentMessage{FileName: proto.String("LT-2026-1008-001.pdf")}}
	got = contextEvent{}
	raw, _ = json.Marshal(nativeMessageEvent(doc, nativeMessageView(doc, "live"), own, ownLID))
	if err := json.Unmarshal(raw, &got); err != nil || got.Kind != "document" || got.Message.Conversation != "LT-2026-1008-001.pdf" || got.Message.FileName != "LT-2026-1008-001.pdf" {
		t.Fatalf("caption-less document: %s", raw)
	}
	doc.Message.DocumentMessage.ContextInfo = &waE2E.ContextInfo{Expiration: proto.Uint32(86400)}
	got = contextEvent{}
	raw, _ = json.Marshal(nativeMessageEvent(doc, nativeMessageView(doc, "live"), own, ownLID))
	if err := json.Unmarshal(raw, &got); err != nil || got.Message.Conversation != "" || got.Message.FileName != "" {
		t.Fatalf("disappearing document: %s", raw)
	}
	// A live edit stays a ProtocolMessage envelope after whatsmeow's unwrap.
	edit := testNativeMessage("")
	edit.IsEdit = true
	edit.Message = &waE2E.Message{ProtocolMessage: &waE2E.ProtocolMessage{Type: waE2E.ProtocolMessage_MESSAGE_EDIT.Enum(), Key: &waCommon.MessageKey{ID: proto.String("3EB0SENT")}, EditedMessage: &waE2E.Message{DocumentMessage: &waE2E.DocumentMessage{Caption: proto.String("new caption"), FileName: proto.String("LT-2026-1008-001.pdf")}}}}
	got = contextEvent{}
	raw, _ = json.Marshal(nativeMessageEvent(edit, nativeMessageView(edit, "live"), own, ownLID))
	if err := json.Unmarshal(raw, &got); err != nil || got.Info.ID != "3EB0SENT" || !got.Info.Edited || got.Kind != "document" || got.Message.Conversation != "new caption" || got.Message.FileName != "LT-2026-1008-001.pdf" {
		t.Fatalf("edited document: %s", raw)
	}
	for _, tc := range []struct {
		name, author string
		keepsActor   bool
	}{{"admin revoke", "555555555@lid", false}, {"own revoke", "987654321@lid", true}} {
		revoke := testNativeMessage("")
		revoke.Info.MessageSource = types.MessageSource{Chat: types.NewJID("120363000000000001", types.GroupServer), Sender: types.NewJID("987654321", types.HiddenUserServer), SenderAlt: types.NewJID("5511988887777", types.DefaultUserServer), IsGroup: true}
		revoke.Info.PushName = "Ana"
		revoke.Message = &waE2E.Message{ProtocolMessage: &waE2E.ProtocolMessage{Type: waE2E.ProtocolMessage_REVOKE.Enum(), Key: &waCommon.MessageKey{ID: proto.String("3EB0GONE"), Participant: proto.String(tc.author)}}}
		got = contextEvent{}
		raw, _ = json.Marshal(nativeMessageEvent(revoke, nativeMessageView(revoke, "live"), own, ownLID))
		if err := json.Unmarshal(raw, &got); err != nil || got.Kind != "deleted" || got.Info.Source.Sender != tc.author || (got.Info.Source.SenderAlt != nil) != tc.keepsActor || (got.Info.PushName == "Ana") != tc.keepsActor {
			t.Fatalf("%s: %s", tc.name, raw)
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
