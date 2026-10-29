package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"log"
	"math"
	"regexp"
	"slices"
	"sync"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
)

type Event struct {
	SessionID   string `json:"session_id"`
	Sequence    uint64 `json:"sequence"`
	Kind        string `json:"kind"`
	TimestampMS int64  `json:"timestamp_ms"`
	Data        any    `json:"data"`
	owner       *Session
	epoch       uint64
}
type EventPage struct {
	Events       []Event `json:"events"`
	NextSequence uint64  `json:"next_sequence"`
	Truncated    bool    `json:"truncated"`
}

func (m *Manager) emit(s *Session, kind string, data any) error {
	limit := m.Config().MaxEvents
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.nextSequence == s.reservedUntil {
		meta := s.meta
		meta.NextSequence = s.nextSequence + 1024
		if meta.NextSequence < s.nextSequence {
			return errors.New("event sequence exhausted")
		}
		if err := writeMetadata(s.dir, meta); err != nil {
			return err
		}
		s.reservedUntil = meta.NextSequence
	}
	e := Event{SessionID: s.meta.SessionID, Sequence: s.nextSequence, Kind: kind, TimestampMS: time.Now().UnixMilli(), Data: data, owner: s, epoch: s.eventEpoch}
	b, err := json.Marshal(e)
	if err != nil {
		return err
	}
	if len(b) > 64*1024 {
		e.Data = map[string]any{"content_omitted": true}
	}
	s.nextSequence++
	s.events = append(s.events, e)
	if len(s.events) > limit {
		s.events = s.events[len(s.events)-limit:]
	}
	select {
	case m.EventQueue <- e:
	default:
	} // Poll exposes the authoritative bounded cursor stream.
	return nil
}
func (m *Manager) Poll(id string, after uint64, limit int) (EventPage, error) {
	ctx, cancel := context.WithTimeout(context.Background(), inboxBackgroundTimeout)
	defer cancel()
	return m.PollContext(ctx, id, after, limit)
}
func (m *Manager) PollContext(ctx context.Context, id string, after uint64, limit int) (EventPage, error) {
	s, err := m.sessionContext(ctx, id)
	if err != nil {
		return EventPage{}, err
	}
	if limit < 1 || limit > 1000 {
		return EventPage{}, errors.New("limit must be 1..1000")
	}
	if err := lockSessionContext(ctx, &s.mu); err != nil {
		return EventPage{}, err
	}
	defer s.mu.Unlock()
	p := EventPage{Events: []Event{}, NextSequence: after}
	p.Truncated = s.nextSequence > 0 && after < s.nextSequence-1
	if len(s.events) > 0 {
		p.Truncated = s.events[0].Sequence > 0 && after < s.events[0].Sequence-1
	}
	for _, e := range s.events {
		if e.Sequence > after {
			p.Events = append(p.Events, e)
			p.NextSequence = e.Sequence
			if len(p.Events) >= limit {
				break
			}
		}
	}
	if len(p.Events) == 0 && s.nextSequence > 0 && s.nextSequence-1 > p.NextSequence {
		p.NextSequence = s.nextSequence - 1
	}
	return p, nil
}

func nativeMessageView(e *events.Message, source string) MessageView {
	info := e.Info
	msg := e.Message
	// ParseWebMessage unwraps protocol edits without preserving IsEdit. Recover
	// the original edit envelope so its target key and edit status remain intact.
	if raw := e.SourceWebMsg.GetMessage(); raw.GetProtocolMessage().GetType() == waE2E.ProtocolMessage_MESSAGE_EDIT {
		msg = raw
	}
	v := MessageView{MessageID: info.ID, ChatID: info.Chat.ToNonAD().String(), SenderID: info.Sender.ToNonAD().String(), FromMe: info.IsFromMe, TimestampMS: info.Timestamp.UnixMilli(), Kind: "unknown", Source: source, Edited: e.IsEdit}
	if info.PushName != "" {
		name := info.PushName
		v.SenderName = &name
	}
	if info.IsFromMe {
		status := "sent"
		v.Status = &status
	}
	if e.IsViewOnce || e.IsViewOnceV2 || e.IsViewOnceV2Extension || msg.GetImageMessage().GetViewOnce() || msg.GetVideoMessage().GetViewOnce() || msg.GetAudioMessage().GetViewOnce() || msg.GetExtendedTextMessage().GetViewOnce() {
		v.Kind = "view_once"
		return v
	}
	if msg == nil {
		return v
	}
	// Protocol edits/revokes address the original message, not the envelope ID.
	if p := msg.GetProtocolMessage(); p != nil {
		if p.GetType() == waE2E.ProtocolMessage_REVOKE {
			v.MessageID = p.GetKey().GetID()
			v.FromMe = p.GetKey().GetFromMe()
			if sender := p.GetKey().GetParticipant(); sender != "" {
				v.SenderID = sender
			}
			v.Kind = "deleted"
			return v
		}
		if p.GetType() == waE2E.ProtocolMessage_MESSAGE_EDIT && p.GetEditedMessage() != nil {
			v.MessageID = p.GetKey().GetID()
			v.FromMe = p.GetKey().GetFromMe()
			if sender := p.GetKey().GetParticipant(); sender != "" {
				v.SenderID = sender
			}
			v.Edited = true
			msg = p.GetEditedMessage()
		}
	}
	if msg.GetImageMessage().GetViewOnce() || msg.GetVideoMessage().GetViewOnce() || msg.GetAudioMessage().GetViewOnce() || msg.GetExtendedTextMessage().GetViewOnce() {
		v.Kind = "view_once"
		return v
	}
	var contextInfo *waE2E.ContextInfo
	switch {
	case msg.Conversation != nil:
		v.Kind = "text"
		v.Text = msg.GetConversation()
	case msg.ExtendedTextMessage != nil:
		v.Kind = "text"
		v.Text = msg.GetExtendedTextMessage().GetText()
		contextInfo = msg.GetExtendedTextMessage().GetContextInfo()
	case msg.ImageMessage != nil:
		v.Kind = "image"
		v.Text = msg.GetImageMessage().GetCaption()
		contextInfo = msg.GetImageMessage().GetContextInfo()
	case msg.VideoMessage != nil:
		v.Kind = "video"
		v.Text = msg.GetVideoMessage().GetCaption()
		contextInfo = msg.GetVideoMessage().GetContextInfo()
	case msg.AudioMessage != nil:
		v.Kind = "audio"
		contextInfo = msg.GetAudioMessage().GetContextInfo()
	case msg.DocumentMessage != nil:
		v.Kind = "document"
		v.Text = msg.GetDocumentMessage().GetCaption()
		v.FileName = msg.GetDocumentMessage().GetFileName()
		if v.Text == "" {
			v.Text = v.FileName
		}
		contextInfo = msg.GetDocumentMessage().GetContextInfo()
	case msg.StickerMessage != nil:
		v.Kind = "sticker"
		contextInfo = msg.GetStickerMessage().GetContextInfo()
	case msg.ContactMessage != nil:
		v.Kind = "contact"
		v.Text = msg.GetContactMessage().GetDisplayName()
		contextInfo = msg.GetContactMessage().GetContextInfo()
	case msg.ContactsArrayMessage != nil:
		v.Kind = "contact"
		v.Text = msg.GetContactsArrayMessage().GetDisplayName()
		contextInfo = msg.GetContactsArrayMessage().GetContextInfo()
	case msg.LocationMessage != nil:
		v.Kind = "location"
		v.Text = msg.GetLocationMessage().GetName()
		contextInfo = msg.GetLocationMessage().GetContextInfo()
	case msg.ReactionMessage != nil:
		v.Kind = "reaction"
		v.Text = msg.GetReactionMessage().GetText()
	case msg.PollCreationMessage != nil:
		v.Kind = "poll"
		v.Text = msg.GetPollCreationMessage().GetName()
		contextInfo = msg.GetPollCreationMessage().GetContextInfo()
	case msg.PollCreationMessageV2 != nil:
		v.Kind = "poll"
		v.Text = msg.GetPollCreationMessageV2().GetName()
		contextInfo = msg.GetPollCreationMessageV2().GetContextInfo()
	case msg.PollCreationMessageV3 != nil:
		v.Kind = "poll"
		v.Text = msg.GetPollCreationMessageV3().GetName()
		contextInfo = msg.GetPollCreationMessageV3().GetContextInfo()
	}
	expiration := contextInfo.GetExpiration()
	start := v.TimestampMS
	unknownExpiry := e.IsEphemeral
	if web := e.SourceWebMsg; web != nil {
		if duration := web.GetEphemeralDuration(); duration > 0 {
			expiration = duration
		}
		if web.EphemeralStartTimestamp != nil {
			unknownExpiry = true
			seconds := web.GetEphemeralStartTimestamp()
			if seconds <= math.MaxInt64/1000 {
				start = int64(seconds) * 1000
			} else {
				expiration = 0
			}
		}
		if web.EphemeralExpirationTimestamp != nil {
			unknownExpiry = true
			seconds := web.GetEphemeralExpirationTimestamp()
			if seconds <= math.MaxInt64/1000 {
				expires := int64(seconds) * 1000
				v.ExpiresAtMS = &expires
			}
		}
	}
	if expiration > 0 {
		delta := int64(expiration) * 1000
		unknownExpiry = true
		if start >= 0 && start <= math.MaxInt64-delta {
			expires := start + delta
			if v.ExpiresAtMS == nil || expires < *v.ExpiresAtMS {
				v.ExpiresAtMS = &expires
			}
		}
	}
	if v.ExpiresAtMS != nil && *v.ExpiresAtMS <= time.Now().UnixMilli() {
		v.Kind = "expired"
		v.Text, v.FileName = "", ""
	} else if unknownExpiry && v.ExpiresAtMS == nil {
		v.Kind = "ephemeral"
		v.Text, v.FileName = "", ""
	} else if v.ExpiresAtMS == nil {
		// Disappearing messages keep no context, so a quote cannot leak them.
		v.Context = contextInfo
	}
	return v
}

// nativeMessageEvent projects message.received data. own and ownLID identify the
// linked account; either may be empty before login.
func nativeMessageEvent(e *events.Message, v MessageView, own, ownLID types.JID) map[string]any {
	isMe := func(raw string) bool {
		j, err := types.ParseJID(raw)
		if err != nil || j.IsEmpty() {
			return false
		}
		j = j.ToNonAD()
		return j == own.ToNonAD() || (!ownLID.IsEmpty() && j == ownLID.ToNonAD())
	}
	orNil := func(value string) any {
		if value == "" {
			return nil
		}
		return value
	}
	preview, fileName := v.Text, v.FileName
	if v.ExpiresAtMS != nil {
		preview, fileName = "", ""
	}
	ci := v.Context
	mentioned := append([]string{}, ci.GetMentionedJID()...)
	mentionedMe := []string{}
	for _, jid := range mentioned {
		if isMe(jid) {
			mentionedMe = append(mentionedMe, jid)
		}
	}
	var quoted any
	if ci.GetStanzaID() != "" {
		// whatsmeow unwraps only the top-level message; a quote keeps its
		// document-with-caption, ephemeral or view-once wrapper.
		qe := (&events.Message{Info: types.MessageInfo{Timestamp: e.Info.Timestamp}, RawMessage: ci.GetQuotedMessage()}).UnwrapRaw()
		q := nativeMessageView(qe, "quoted")
		if q.ExpiresAtMS != nil {
			q.Text, q.FileName = "", ""
		}
		quoted = map[string]any{"id": ci.GetStanzaID(), "participant": ci.GetParticipant(), "is_me": isMe(ci.GetParticipant()), "kind": q.Kind, "text": clipInboxText(q.Text, 4096), "file_name": q.FileName}
	}
	// A revoke or edit reports the original author as sender. When that is not
	// the envelope's sender (a group admin's revoke), the envelope's alternate
	// JID and push name are someone else's.
	senderAlt, pushName := e.Info.SenderAlt.ToNonAD().String(), e.Info.PushName
	if v.SenderID != e.Info.Sender.ToNonAD().String() {
		senderAlt, pushName = "", ""
	}
	source := map[string]any{"chat": v.ChatID, "sender": v.SenderID, "sender_alt": orNil(senderAlt), "is_from_me": v.FromMe, "is_group": e.Info.IsGroup, "addressing_mode": orNil(string(e.Info.AddressingMode))}
	return map[string]any{
		"info":    map[string]any{"id": v.MessageID, "timestamp_ms": v.TimestampMS, "push_name": pushName, "edited": v.Edited, "source": source},
		"message": map[string]any{"conversation": preview, "mentioned_jids": mentioned, "mentions_me": len(mentionedMe) > 0, "mentioned_me_jids": mentionedMe, "file_name": fileName, "quoted": quoted},
		"kind":    v.Kind,
	}
}
func (m *Manager) handleNativeEvent(s *Session, client *whatsmeow.Client, capture *InboxCapture, event any) bool {
	s.callbacks.Lock()
	defer s.callbacks.Unlock()
	s.mu.Lock()
	active := s.client == client && s.acceptEvents
	s.mu.Unlock()
	if !active {
		return true
	}
	var err error
	switch e := event.(type) {
	case *events.Connected:
		if client.Store.ID != nil {
			err = refreshNativeAccount(s, capture, client.Store.ID.ToNonAD().String())
		}
		if err == nil {
			s.mu.Lock()
			if s.client == client {
				s.status = "connected"
				s.qr = nil
				s.pairCode = nil
				s.pairPhone = ""
			}
			s.mu.Unlock()
			err = m.emit(s, "session.connected", map[string]any{})
		}
	case *events.Disconnected:
		s.mu.Lock()
		if s.client == client {
			s.status = "disconnected"
			s.qr = nil
		}
		s.mu.Unlock()
		err = m.emit(s, "session.disconnected", map[string]any{})
	case *events.PairSuccess:
		err = refreshNativeAccount(s, capture, e.ID.ToNonAD().String())
		if err == nil {
			err = m.emit(s, "session.paired", map[string]any{})
		}
	case *events.LoggedOut:
		s.mu.Lock()
		if s.client == client {
			s.status = "logged_out"
			s.acceptEvents = false
			s.qr = nil
			s.pairCode = nil
			s.pairPhone = ""
			s.events = []Event{}
			s.eventEpoch++
		}
		s.mu.Unlock()
		err = s.inbox.Clear()
		if err == nil {
			err = m.emit(s, "session.logged_out", map[string]any{})
		}
	case *events.PairError, *events.ConnectFailure, *events.ClientOutdated:
		s.mu.Lock()
		if s.client == client {
			s.status = "failed"
			s.acceptEvents = false
			s.qr = nil
			s.pairCode = nil
		}
		s.mu.Unlock()
		err = m.emit(s, "session.failed", map[string]any{"reason": "authentication or protocol failure"})
	case *events.Message:
		view := nativeMessageView(e, "live")
		if view.MessageID == "" || view.ChatID == "" || view.Kind == "unknown" {
			return true
		}
		if _, parseErr := canonicalInboxChat(view.ChatID); parseErr != nil {
			return true
		}
		err = capture.Record([]MessageView{view})
		if err == nil {
			err = m.emit(s, "message.received", nativeMessageEvent(e, view, client.Store.GetJID(), client.Store.GetLID()))
		}
	case *events.HistorySync:
		var count int
		count, err = captureNativeHistory(client, capture, e)
		progress := int(e.Data.GetProgress())
		if err == nil {
			err = m.emit(s, "history.sync", map[string]any{"messages": count, "progress": progress})
		}
	case *events.Receipt:
		status := ""
		switch string(e.Type) {
		case "":
			status = "delivered"
		case "read", "read-self":
			status = "read"
		case "played", "played-self":
			status = "played"
		}
		if status != "" {
			err = capture.RecordReceipt(e.Chat.ToNonAD().String(), e.MessageIDs, status)
		}
		if err == nil {
			err = m.emit(s, "message.receipt", map[string]any{"chat_id": e.Chat.ToNonAD().String(), "message_ids": e.MessageIDs, "status": status})
		}
	case *events.GroupInfo:
		err = m.emit(s, "group.updated", map[string]any{"group": e.JID.String(), "joined": e.Join, "left": e.Leave, "promoted": e.Promote, "demoted": e.Demote})
	case *events.JoinedGroup:
		err = m.emit(s, "group.joined", map[string]any{"group": e.JID.String()})
	case *events.ChatPresence:
		err = m.emit(s, "chat.presence", map[string]any{"chat_id": e.Chat.String(), "sender": e.Sender.String(), "state": e.State})
	}
	if err != nil {
		log.Print("WhatsApp event could not be persisted; delivery was not acknowledged")
	}
	return err == nil
}

// Caller serializes callbacks. An unexpected account change must invalidate
// both the archive capture and any buffered events from the previous identity.
func refreshNativeAccount(s *Session, capture *InboxCapture, account string) error {
	if err := s.inbox.SetAccount(account); err != nil {
		return err
	}
	next := s.inbox.Capture()
	if next.generation != capture.generation {
		s.mu.Lock()
		s.events = []Event{}
		s.eventEpoch++
		s.mu.Unlock()
	}
	*capture = *next
	return nil
}

func captureNativeHistory(client *whatsmeow.Client, capture *InboxCapture, event *events.HistorySync) (int, error) {
	batch := make([]MessageView, 0, 256)
	titles := map[string]string{}
	count := 0
	flush := func(progress *int) error {
		if err := capture.RecordHistory(batch, titles, progress); err != nil {
			return err
		}
		count += len(batch)
		batch = batch[:0]
		titles = map[string]string{}
		return nil
	}
	for _, conversation := range event.Data.GetConversations() {
		chat, err := parseJID(conversation.GetID())
		if err != nil {
			continue
		}
		if _, err = canonicalInboxChat(chat.String()); err != nil {
			continue
		}
		if name := conversation.GetName(); name != "" {
			titles[chat.String()] = name
		}
		for _, entry := range conversation.GetMessages() {
			parsed, err := client.ParseWebMessage(chat, entry.GetMessage())
			if err != nil {
				continue
			}
			view := nativeMessageView(parsed, "history")
			if view.MessageID == "" || view.Kind == "unknown" {
				continue
			}
			batch = append(batch, view)
			if len(batch) == cap(batch) {
				if err = flush(nil); err != nil {
					return count, err
				}
			}
		}
		if len(titles) >= 256 {
			if err = flush(nil); err != nil {
				return count, err
			}
		}
	}
	progress := int(event.Data.GetProgress())
	err := flush(&progress)
	return count, err
}

// Events already queued before logout or deletion cannot enter a new account's stream.
func (m *Manager) eventCurrent(event Event) bool {
	m.mu.RLock()
	defer m.mu.RUnlock()
	if m.closed || m.sessions[event.SessionID] != event.owner || event.owner == nil {
		return false
	}
	event.owner.mu.Lock()
	defer event.owner.mu.Unlock()
	return event.epoch == event.owner.eventEpoch
}

type EventFilter struct {
	SessionID string   `json:"session_id"`
	Kinds     []string `json:"kinds"`
}
type eventBinding struct {
	filter EventFilter
	config iii.TriggerConfig
}
type EventRouter struct {
	mu       sync.RWMutex
	bindings map[string]eventBinding
}

var eventKindPattern = regexp.MustCompile(`^[A-Za-z0-9._]{1,128}$`)

func (r *EventRouter) RegisterTrigger(_ context.Context, b iii.TriggerConfig) error {
	var f EventFilter
	d := json.NewDecoder(bytes.NewReader(b.Config))
	d.DisallowUnknownFields()
	if err := d.Decode(&f); err != nil {
		return errors.New("invalid WhatsApp event filter")
	}
	var trailing any
	if err := d.Decode(&trailing); err != io.EOF {
		return errors.New("invalid WhatsApp event filter")
	}
	if f.SessionID != "" && validSessionID(f.SessionID) != nil {
		return errors.New("invalid session_id filter")
	}
	if len(f.Kinds) > 128 {
		return errors.New("too many event kinds")
	}
	for _, kind := range f.Kinds {
		if !eventKindPattern.MatchString(kind) {
			return errors.New("invalid event kind")
		}
	}
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.bindings == nil {
		r.bindings = map[string]eventBinding{}
	}
	if _, exists := r.bindings[b.ID]; !exists && len(r.bindings) >= 1024 {
		return errors.New("event subscription limit reached")
	}
	r.bindings[b.ID] = eventBinding{f, b}
	return nil
}
func (r *EventRouter) UnregisterTrigger(_ context.Context, b iii.TriggerConfig) error {
	r.mu.Lock()
	defer r.mu.Unlock()
	delete(r.bindings, b.ID)
	return nil
}
func registerEvents(ctx context.Context, client *iii.Client, m *Manager) error {
	r := &EventRouter{bindings: map[string]eventBinding{}}
	if err := client.RegisterTriggerType("whatsapp::events", "Native WhatsApp session, message, receipt and group events", r, iii.RegisterTriggerTypeOptions{
		TriggerRequestFormat: nativeEventFilterSchema,
		CallRequestFormat:    nativeEventPayloadSchema,
	}); err != nil {
		return err
	}
	go func() {
		for {
			select {
			case <-ctx.Done():
				return
			case e := <-m.EventQueue:
				if !m.eventCurrent(e) {
					continue
				}
				r.mu.RLock()
				bindings := []iii.TriggerConfig{}
				for _, b := range r.bindings {
					if (b.filter.SessionID == "" || b.filter.SessionID == e.SessionID) && (len(b.filter.Kinds) == 0 || slices.Contains(b.filter.Kinds, e.Kind)) {
						bindings = append(bindings, b.config)
					}
				}
				r.mu.RUnlock()
				data, _ := json.Marshal(e)
				for _, b := range bindings {
					if !m.eventCurrent(e) {
						break
					}
					callCtx, cancel := context.WithTimeout(ctx, 5*time.Second)
					_, err := client.Trigger(callCtx, iii.TriggerRequest{FunctionID: b.FunctionID, Data: data, Namespace: b.Namespace, Metadata: b.Metadata, Timeout: 5 * time.Second})
					cancel()
					if err != nil && ctx.Err() == nil {
						log.Print("WhatsApp event delivery failed; reconcile with events::poll")
					}
				}
			}
		}
	}()
	return nil
}
