package main

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"strings"
	"time"
	"unicode"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waCommon"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
)

func nativeText(value, name string, max int, optional bool) error {
	if (!optional && strings.TrimSpace(value) == "") || len(value) > max || strings.ContainsRune(value, 0) {
		return fmt.Errorf("%s must be %s and at most %d bytes without NUL", name, map[bool]string{true: "valid text", false: "nonempty"}[optional], max)
	}
	return nil
}

func nativeMessageID(id string) error {
	if id == "" || len(id) > 200 || strings.IndexFunc(id, unicode.IsSpace) >= 0 || strings.ContainsRune(id, 0) {
		return errors.New("message_id must contain 1 to 200 bytes without whitespace or NUL")
	}
	return nil
}

func nativePerson(jid types.JID) bool {
	return jid.Server == types.DefaultUserServer || jid.Server == types.HiddenUserServer
}
func nativeChat(jid types.JID) bool { return nativePerson(jid) || jid.Server == types.GroupServer }
func nativeMap(m map[string]any, key string) map[string]any {
	v, _ := m[key].(map[string]any)
	return v
}
func nativeNumber(m map[string]any, key string) float64 {
	switch n := m[key].(type) {
	case float64:
		return n
	case int:
		return float64(n)
	case int64:
		return float64(n)
	case json.Number:
		value, _ := n.Float64()
		return value
	}
	return 0
}

func nativeKey(m map[string]any) (*waCommon.MessageKey, types.JID, error) {
	chat, err := parseJID(stringArg(m, "chat_id"))
	if err != nil {
		return nil, chat, err
	}
	id := stringArg(m, "message_id")
	if err = nativeMessageID(id); err != nil {
		return nil, chat, err
	}
	fromMe := boolArg(m, "from_me")
	key := &waCommon.MessageKey{RemoteJID: proto.String(chat.String()), ID: proto.String(id), FromMe: proto.Bool(fromMe)}
	participant := stringArg(m, "participant_id")
	if participant != "" {
		jid, e := parseJID(participant)
		if e != nil {
			return nil, chat, e
		}
		if !nativePerson(jid) {
			return nil, chat, errors.New("participant_id must identify a person")
		}
		key.Participant = proto.String(jid.String())
	}
	if (chat.Server == types.GroupServer && !fromMe || chat == types.StatusBroadcastJID) && participant == "" {
		return nil, chat, errors.New("participant_id is required for this message")
	}
	return key, chat, nil
}

func nativeSent(response whatsmeow.SendResponse, chat types.JID) map[string]any {
	if !response.Chat.IsEmpty() {
		chat = response.Chat
	}
	return map[string]any{"message_id": response.ID, "chat_id": chat.String(), "recipient_fanout": nil}
}

func nativeRecordSent(m *Manager, session string, client *whatsmeow.Client, response whatsmeow.SendResponse, chat types.JID, text, kind string) error {
	sender := response.Sender
	if sender.IsEmpty() && client.Store.ID != nil {
		sender = *client.Store.ID
	}
	if !response.Chat.IsEmpty() {
		chat = response.Chat
	}
	status := "submitted"
	err := m.RecordSent(session, MessageView{MessageID: response.ID, ChatID: chat.String(), SenderID: sender.String(), FromMe: true, TimestampMS: response.Timestamp.UnixMilli(), Kind: kind, Text: text, Source: "sent", Status: &status})
	if err != nil {
		return fmt.Errorf("message %s was accepted but could not be saved locally; do not resend automatically", response.ID)
	}
	return nil
}

func nativeMessagePayload(client *whatsmeow.Client, action string, args map[string]any, chat types.JID) (*waE2E.Message, error) {
	if !nativeChat(chat) && chat.Server != types.NewsletterServer {
		return nil, errors.New("message sending requires a person, group or newsletter; status publishing is not supported")
	}
	text := stringArg(args, "text")
	switch action {
	case "send_text", "reply", "edit":
		if err := nativeText(text, "text", 65536, false); err != nil {
			return nil, err
		}
		if action == "send_text" {
			return &waE2E.Message{Conversation: proto.String(text)}, nil
		}
		if err := nativeMessageID(stringArg(args, "message_id")); err != nil {
			return nil, err
		}
		if action == "edit" {
			if chat.Server == types.NewsletterServer {
				return nil, errors.New("use newsletters::edit for newsletters")
			}
			return client.BuildEdit(chat, stringArg(args, "message_id"), &waE2E.Message{Conversation: proto.String(text)}), nil
		}
		sender, err := parseJID(stringArg(args, "sender_id"))
		if err != nil {
			return nil, err
		}
		if !nativePerson(sender) {
			return nil, errors.New("sender_id must identify a person")
		}
		quote := stringArg(args, "quoted_text")
		if err = nativeText(quote, "quoted_text", 65536, false); err != nil {
			return nil, err
		}
		return &waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{Text: proto.String(text), ContextInfo: &waE2E.ContextInfo{StanzaID: proto.String(stringArg(args, "message_id")), Participant: proto.String(sender.String()), RemoteJID: proto.String(chat.String()), QuotedMessage: &waE2E.Message{Conversation: proto.String(quote)}}}}, nil
	case "delete":
		if chat.Server == types.NewsletterServer {
			return nil, errors.New("use newsletters::revoke for newsletters")
		}
		if err := nativeMessageID(stringArg(args, "message_id")); err != nil {
			return nil, err
		}
		var sender types.JID
		if raw := stringArg(args, "original_sender_id"); raw != "" {
			var err error
			sender, err = parseJID(raw)
			if err != nil {
				return nil, err
			}
			if chat.Server != types.GroupServer || !nativePerson(sender) {
				return nil, errors.New("original_sender_id requires a group and a person")
			}
		}
		return client.BuildRevoke(chat, sender, stringArg(args, "message_id")), nil
	case "send_location":
		lat, lon := nativeNumber(args, "latitude"), nativeNumber(args, "longitude")
		if math.IsNaN(lat) || math.IsNaN(lon) || math.IsInf(lat, 0) || math.IsInf(lon, 0) || lat < -90 || lat > 90 || lon < -180 || lon > 180 {
			return nil, errors.New("latitude or longitude is out of range")
		}
		name, address := stringArg(args, "name"), stringArg(args, "address")
		if err := nativeText(name, "name", 1024, true); err != nil {
			return nil, err
		}
		if err := nativeText(address, "address", 4096, true); err != nil {
			return nil, err
		}
		return &waE2E.Message{LocationMessage: &waE2E.LocationMessage{DegreesLatitude: proto.Float64(lat), DegreesLongitude: proto.Float64(lon), Name: proto.String(name), Address: proto.String(address)}}, nil
	case "send_contact":
		name, vcard := stringArg(args, "display_name"), stringArg(args, "vcard")
		if err := nativeText(name, "display_name", 1024, false); err != nil {
			return nil, err
		}
		if err := nativeText(vcard, "vcard", 65536, false); err != nil {
			return nil, err
		}
		if !strings.HasPrefix(vcard, "BEGIN:VCARD") || !strings.HasSuffix(strings.TrimSpace(vcard), "END:VCARD") {
			return nil, errors.New("vcard must begin with BEGIN:VCARD and end with END:VCARD")
		}
		return &waE2E.Message{ContactMessage: &waE2E.ContactMessage{DisplayName: proto.String(name), Vcard: proto.String(vcard)}}, nil
	}
	return nil, errors.New("unknown message action")
}

func nativePollOptions(options []string, min int) error {
	if len(options) < min || len(options) > 12 {
		return fmt.Errorf("provide %d to 12 poll options", min)
	}
	seen := map[string]bool{}
	for _, option := range options {
		if err := nativeText(option, "poll option", 100, false); err != nil {
			return err
		}
		if seen[option] {
			return errors.New("poll options must be unique")
		}
		seen[option] = true
	}
	return nil
}

func registerMessaging(r *Registry) {
	for _, action := range []string{"send_text", "reply", "edit", "delete", "send_location", "send_contact"} {
		r.Add("whatsapp::messages::"+action, func(ctx context.Context, args map[string]any) (any, error) {
			chat, err := parseJID(stringArg(args, "chat_id"))
			if err != nil {
				return nil, err
			}
			client, err := r.Manager.Client(stringArg(args, "session_id"))
			if err != nil {
				return nil, err
			}
			payload, err := nativeMessagePayload(client, action, args, chat)
			if err != nil {
				return nil, err
			}
			result, err := client.SendMessage(ctx, chat, payload)
			if err != nil {
				return nil, err
			}
			if action == "send_text" || action == "reply" {
				if err = nativeRecordSent(r.Manager, stringArg(args, "session_id"), client, result, chat, stringArg(args, "text"), "text"); err != nil {
					return nil, err
				}
			}
			return nativeSent(result, chat), nil
		})
	}
	r.Add("whatsapp::messages::react", func(ctx context.Context, args map[string]any) (any, error) {
		key, chat, err := nativeKey(nativeMap(args, "target"))
		if err != nil {
			return nil, err
		}
		if chat.Server == types.NewsletterServer {
			return nil, errors.New("use newsletters::react for newsletters")
		}
		if !nativeChat(chat) {
			return nil, errors.New("reactions require a person or group")
		}
		if chat.Server == types.GroupServer && key.GetParticipant() == "" {
			return nil, errors.New("participant_id is required for group reactions")
		}
		emoji := stringArg(args, "emoji")
		if err = nativeText(emoji, "emoji", 64, true); err != nil {
			return nil, err
		}
		client, err := r.Manager.Client(stringArg(args, "session_id"))
		if err != nil {
			return nil, err
		}
		result, err := client.SendMessage(ctx, chat, &waE2E.Message{ReactionMessage: &waE2E.ReactionMessage{Key: key, Text: proto.String(emoji), SenderTimestampMS: proto.Int64(time.Now().UnixMilli())}})
		if err != nil {
			return nil, err
		}
		return nativeSent(result, chat), nil
	})
	r.Add("whatsapp::messages::send_poll", func(ctx context.Context, args map[string]any) (any, error) {
		name, options := stringArg(args, "name"), stringsArg(args, "options")
		if err := nativeText(name, "name", 255, false); err != nil {
			return nil, err
		}
		if err := nativePollOptions(options, 2); err != nil {
			return nil, err
		}
		count := intArg(args, "selectable_count", 1)
		if count < 1 || count > len(options) {
			return nil, errors.New("selectable_count must be between 1 and the option count")
		}
		chat, err := parseJID(stringArg(args, "chat_id"))
		if err != nil {
			return nil, err
		}
		if !nativeChat(chat) {
			return nil, errors.New("polls require a person or group")
		}
		client, err := r.Manager.Client(stringArg(args, "session_id"))
		if err != nil {
			return nil, err
		}
		payload := client.BuildPollCreation(name, options, count)
		result, err := client.SendMessage(ctx, chat, payload)
		if err != nil {
			return nil, err
		}
		return map[string]any{"message": nativeSent(result, chat), "message_secret_base64": base64.StdEncoding.EncodeToString(payload.GetMessageContextInfo().GetMessageSecret())}, nil
	})
	r.Add("whatsapp::messages::vote_poll", func(ctx context.Context, args map[string]any) (any, error) {
		chat, err := parseJID(stringArg(args, "chat_id"))
		if err != nil {
			return nil, err
		}
		if !nativeChat(chat) {
			return nil, errors.New("polls require a person or group")
		}
		creator, err := parseJID(stringArg(args, "creator_id"))
		if err != nil {
			return nil, err
		}
		if !nativePerson(creator) {
			return nil, errors.New("creator_id must identify a person")
		}
		id := stringArg(args, "message_id")
		if err = nativeMessageID(id); err != nil {
			return nil, err
		}
		options := stringsArg(args, "option_names")
		if err = nativePollOptions(options, 0); err != nil {
			return nil, err
		}
		secret, err := decodeMedia(stringArg(args, "message_secret_base64"), 32)
		if err != nil || len(secret) != 32 {
			return nil, errors.New("message_secret_base64 must encode 32 bytes")
		}
		client, err := r.Manager.Client(stringArg(args, "session_id"))
		if err != nil {
			return nil, err
		}
		existing, _, err := client.Store.MsgSecrets.GetMessageSecret(ctx, chat, creator, id)
		if err != nil {
			return nil, err
		}
		if len(existing) > 0 && !bytes.Equal(existing, secret) {
			return nil, errors.New("poll secret does not match the stored message")
		}
		if len(existing) == 0 {
			if err = client.Store.MsgSecrets.PutMessageSecret(ctx, chat, creator, id, secret); err != nil {
				return nil, err
			}
		}
		own := client.Store.ID != nil && creator.ToNonAD() == client.Store.ID.ToNonAD() || creator.ToNonAD() == client.Store.LID.ToNonAD()
		payload, err := client.BuildPollVote(ctx, &types.MessageInfo{MessageSource: types.MessageSource{Chat: chat, Sender: creator, IsFromMe: own, IsGroup: chat.Server == types.GroupServer}, ID: id}, options)
		if err != nil {
			return nil, err
		}
		result, err := client.SendMessage(ctx, chat, payload)
		if err != nil {
			return nil, err
		}
		return nativeSent(result, chat), nil
	})
	r.Add("whatsapp::calls::reject", func(ctx context.Context, args map[string]any) (any, error) {
		id := stringArg(args, "call_id")
		if err := nativeText(id, "call_id", 256, false); err != nil {
			return nil, err
		}
		peer, err := parseJID(stringArg(args, "peer"))
		if err != nil {
			return nil, err
		}
		creator, err := parseJID(stringArg(args, "call_creator"))
		if err != nil {
			return nil, err
		}
		if !nativePerson(peer) || !nativePerson(creator) || peer.ToNonAD() != creator.ToNonAD() {
			return nil, errors.New("call rejection requires peer and call_creator to identify the same person")
		}
		client, err := r.Manager.Client(stringArg(args, "session_id"))
		if err != nil {
			return nil, err
		}
		if err = client.RejectCall(ctx, creator, id); err != nil {
			return nil, err
		}
		return applied(), nil
	})
	registerNativeChats(r)
	registerNativeMedia(r)
	registerNativeNewsletters(r)
}
