package main

import (
	"context"
	"errors"
	"time"

	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/proto/waSyncAction"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
)

func nativeBoolIndex(value bool) string {
	if value {
		return "1"
	}
	return "0"
}
func nativeSeconds(value int64) (*int64, error) {
	if value < 1000 {
		return nil, errors.New("message timestamp must be at least 1000 milliseconds since the Unix epoch")
	}
	return proto.Int64(value / 1000), nil
}
func nativeRange(args map[string]any, chat types.JID) (*waSyncAction.SyncActionMessageRange, error) {
	if args == nil {
		return nil, nil
	}
	ts, err := nativeSeconds(int64(nativeNumber(args, "last_message_timestamp_ms")))
	if err != nil {
		return nil, err
	}
	out := &waSyncAction.SyncActionMessageRange{LastMessageTimestamp: ts}
	if value, ok := args["last_system_message_timestamp_ms"]; ok && value != nil {
		out.LastSystemMessageTimestamp, err = nativeSeconds(int64(nativeNumber(args, "last_system_message_timestamp_ms")))
		if err != nil {
			return nil, err
		}
	}
	items, _ := args["messages"].([]any)
	if len(items) > 256 {
		return nil, errors.New("message range allows at most 256 messages")
	}
	for _, item := range items {
		m, ok := item.(map[string]any)
		if !ok {
			return nil, errors.New("invalid message range")
		}
		key, keyChat, err := nativeKey(nativeMap(m, "key"))
		if err != nil {
			return nil, err
		}
		if keyChat != chat {
			return nil, errors.New("range message belongs to a different chat")
		}
		ts, err := nativeSeconds(int64(nativeNumber(m, "timestamp_ms")))
		if err != nil {
			return nil, err
		}
		out.Messages = append(out.Messages, &waSyncAction.SyncActionMessage{Key: key, Timestamp: ts})
	}
	return out, nil
}

func nativeChatPatch(action string, args map[string]any, chat types.JID) (appstate.PatchInfo, error) {
	switch action {
	case "pin", "unpin":
		return appstate.BuildPin(chat, action == "pin"), nil
	case "mute", "unmute":
		var until *int64
		if value, ok := args["until_timestamp_ms"]; ok && value != nil {
			v := int64(nativeNumber(args, "until_timestamp_ms"))
			if v <= time.Now().UnixMilli() {
				return appstate.PatchInfo{}, errors.New("until_timestamp_ms must be in the future")
			}
			until = &v
		}
		return appstate.BuildMuteAbs(chat, action == "mute", until), nil
	case "lock", "unlock":
		return appstate.PatchInfo{Type: appstate.WAPatchRegularLow, Mutations: []appstate.MutationInfo{{Index: []string{appstate.IndexLock, chat.String()}, Version: 7, Value: &waSyncAction.SyncActionValue{LockChatAction: &waSyncAction.LockChatAction{Locked: proto.Bool(action == "lock")}}}}}, nil
	}
	messageRange, err := nativeRange(nativeMap(args, "message_range"), chat)
	if err != nil {
		return appstate.PatchInfo{}, err
	}
	switch action {
	case "archive", "unarchive":
		p := appstate.BuildArchive(chat, action == "archive", time.Time{}, nil)
		p.Mutations[0].Value.ArchiveChatAction.MessageRange = messageRange
		return p, nil
	case "mark_read":
		p := appstate.BuildMarkChatAsRead(chat, boolArg(args, "read"), time.Time{}, nil)
		p.Mutations[0].Value.MarkChatAsReadAction.MessageRange = messageRange
		return p, nil
	case "delete":
		p := appstate.BuildDeleteChat(chat, time.Time{}, nil, boolArg(args, "delete_media"))
		p.Mutations[0].Value.DeleteChatAction.MessageRange = messageRange
		return p, nil
	case "clear":
		return appstate.PatchInfo{Type: appstate.WAPatchRegularHigh, Mutations: []appstate.MutationInfo{{Index: []string{appstate.IndexClearChat, chat.String(), nativeBoolIndex(boolArg(args, "delete_starred")), nativeBoolIndex(boolArg(args, "delete_media"))}, Version: 6, Value: &waSyncAction.SyncActionValue{ClearChatAction: &waSyncAction.ClearChatAction{MessageRange: messageRange}}}}}, nil
	}
	return appstate.PatchInfo{}, errors.New("unknown chat action")
}

func registerNativeChats(r *Registry) {
	for _, action := range []string{"pin", "unpin", "mute", "unmute", "lock", "unlock", "archive", "unarchive", "mark_read", "delete", "clear"} {
		r.Add("whatsapp::chats::"+action, func(ctx context.Context, args map[string]any) (any, error) {
			chat, err := parseJID(stringArg(args, "chat_id"))
			if err != nil {
				return nil, err
			}
			patch, err := nativeChatPatch(action, args, chat)
			if err != nil {
				return nil, err
			}
			client, err := r.Manager.Client(stringArg(args, "session_id"))
			if err != nil {
				return nil, err
			}
			if err = client.SendAppState(ctx, patch); err != nil {
				return nil, err
			}
			return applied(), nil
		})
	}
	for _, action := range []string{"mark_read", "mark_played"} {
		r.Add("whatsapp::messages::"+action, func(ctx context.Context, args map[string]any) (any, error) {
			chat, err := parseJID(stringArg(args, "chat_id"))
			if err != nil {
				return nil, err
			}
			var sender types.JID
			if raw := stringArg(args, "sender_id"); raw != "" {
				sender, err = parseJID(raw)
				if err != nil {
					return nil, err
				}
				if !nativePerson(sender) {
					return nil, errors.New("sender_id must identify a person")
				}
			}
			if (chat.Server == types.GroupServer || chat == types.StatusBroadcastJID) && sender.IsEmpty() {
				return nil, errors.New("sender_id is required for group or status receipts")
			}
			ids := stringsArg(args, "message_ids")
			if len(ids) < 1 || len(ids) > 256 {
				return nil, errors.New("provide 1 to 256 message_ids")
			}
			for _, id := range ids {
				if err = nativeMessageID(id); err != nil {
					return nil, err
				}
			}
			client, err := r.Manager.Client(stringArg(args, "session_id"))
			if err != nil {
				return nil, err
			}
			receipt := types.ReceiptTypeRead
			if action == "mark_played" {
				receipt = types.ReceiptTypePlayed
			}
			if err = client.MarkRead(ctx, ids, time.Now(), chat, sender, receipt); err != nil {
				return nil, err
			}
			return applied(), nil
		})
	}
	r.Add("whatsapp::chats::presence", func(ctx context.Context, args map[string]any) (any, error) {
		client, err := r.Manager.Client(stringArg(args, "session_id"))
		if err != nil {
			return nil, err
		}
		if err = client.SendPresence(ctx, types.Presence(stringArg(args, "presence"))); err != nil {
			return nil, err
		}
		return applied(), nil
	})
	r.Add("whatsapp::chats::subscribe_presence", func(ctx context.Context, args map[string]any) (any, error) {
		chat, err := parseJID(stringArg(args, "chat_id"))
		if err != nil {
			return nil, err
		}
		if !nativePerson(chat) {
			return nil, errors.New("presence subscriptions require a person")
		}
		client, err := r.Manager.Client(stringArg(args, "session_id"))
		if err != nil {
			return nil, err
		}
		if err = client.SubscribePresence(ctx, chat); err != nil {
			return nil, err
		}
		return applied(), nil
	})
	r.Add("whatsapp::chats::state", func(ctx context.Context, args map[string]any) (any, error) {
		chat, err := parseJID(stringArg(args, "chat_id"))
		if err != nil {
			return nil, err
		}
		client, err := r.Manager.Client(stringArg(args, "session_id"))
		if err != nil {
			return nil, err
		}
		state, media := types.ChatPresence(stringArg(args, "state")), types.ChatPresenceMediaText
		if state == "recording" {
			state, media = types.ChatPresenceComposing, types.ChatPresenceMediaAudio
		}
		if err = client.SendChatPresence(ctx, chat, state, media); err != nil {
			return nil, err
		}
		return applied(), nil
	})
	for _, action := range []string{"star", "unstar", "delete_for_me", "pin", "unpin", "keep"} {
		r.Add("whatsapp::messages::"+action, func(ctx context.Context, args map[string]any) (any, error) {
			key, chat, err := nativeKey(nativeMap(args, "target"))
			if err != nil {
				return nil, err
			}
			if !nativeChat(chat) {
				return nil, errors.New("message actions require a person or group")
			}
			client, err := r.Manager.Client(stringArg(args, "session_id"))
			if err != nil {
				return nil, err
			}
			if action == "star" || action == "unstar" || action == "delete_for_me" {
				participant := key.GetParticipant()
				if participant == "" {
					participant = "0"
				}
				value := &waSyncAction.SyncActionValue{}
				index, version := appstate.IndexStar, int32(2)
				if action == "delete_for_me" {
					index, version = appstate.IndexDeleteMessageForMe, 3
					var ts *int64
					if v, ok := args["message_timestamp_ms"]; ok && v != nil {
						ts, err = nativeSeconds(int64(nativeNumber(args, "message_timestamp_ms")))
						if err != nil {
							return nil, err
						}
					}
					value.DeleteMessageForMeAction = &waSyncAction.DeleteMessageForMeAction{DeleteMedia: proto.Bool(boolArg(args, "delete_media")), MessageTimestamp: ts}
				} else {
					value.StarAction = &waSyncAction.StarAction{Starred: proto.Bool(action == "star")}
				}
				patch := appstate.PatchInfo{Type: appstate.WAPatchRegularHigh, Mutations: []appstate.MutationInfo{{Index: []string{index, chat.String(), key.GetID(), nativeBoolIndex(key.GetFromMe()), participant}, Version: version, Value: value}}}
				if err = client.SendAppState(ctx, patch); err != nil {
					return nil, err
				}
				return applied(), nil
			}
			payload := &waE2E.Message{}
			if action == "keep" {
				kind := waE2E.KeepType_UNDO_KEEP_FOR_ALL
				if boolArg(args, "keep") {
					kind = waE2E.KeepType_KEEP_FOR_ALL
				}
				payload.KeepInChatMessage = &waE2E.KeepInChatMessage{Key: key, KeepType: kind.Enum(), TimestampMS: proto.Int64(time.Now().UnixMilli())}
			} else {
				kind, duration := waE2E.PinInChatMessage_UNPIN_FOR_ALL, uint32(0)
				if action == "pin" {
					kind = waE2E.PinInChatMessage_PIN_FOR_ALL
					switch stringArg(args, "duration") {
					case "hours24":
						duration = 86400
					case "days7":
						duration = 7 * 86400
					case "days30":
						duration = 30 * 86400
					default:
						return nil, errors.New("invalid pin duration")
					}
				}
				payload.PinInChatMessage = &waE2E.PinInChatMessage{Key: key, Type: kind.Enum(), SenderTimestampMS: proto.Int64(time.Now().UnixMilli())}
				payload.MessageContextInfo = &waE2E.MessageContextInfo{MessageAddOnDurationInSecs: proto.Uint32(duration)}
			}
			result, err := client.SendMessage(ctx, chat, payload)
			if err != nil {
				return nil, err
			}
			return nativeSent(result, chat), nil
		})
	}
	for _, action := range []string{"set", "delete", "add_chat", "remove_chat", "add_message", "remove_message"} {
		r.Add("whatsapp::labels::"+action, func(ctx context.Context, args map[string]any) (any, error) {
			id := stringArg(args, "label_id")
			if err := nativeText(id, "label_id", 128, false); err != nil {
				return nil, err
			}
			var patch appstate.PatchInfo
			if action == "set" || action == "delete" {
				name, color := stringArg(args, "name"), intArg(args, "color", 0)
				if action == "set" {
					if err := nativeText(name, "name", 100, false); err != nil {
						return nil, err
					}
					if color < 0 || color > 19 {
						return nil, errors.New("label color must be between 0 and 19")
					}
				}
				patch = appstate.BuildLabelEdit(id, name, int32(color), action == "delete")
			} else {
				chat, err := parseJID(stringArg(args, "chat_id"))
				if err != nil {
					return nil, err
				}
				if action == "add_chat" || action == "remove_chat" {
					patch = appstate.BuildLabelChat(chat, id, action == "add_chat")
				} else {
					messageID := stringArg(args, "message_id")
					if err = nativeMessageID(messageID); err != nil {
						return nil, err
					}
					patch = appstate.BuildLabelMessage(chat, id, messageID, action == "add_message")
				}
			}
			client, err := r.Manager.Client(stringArg(args, "session_id"))
			if err != nil {
				return nil, err
			}
			if err = client.SendAppState(ctx, patch); err != nil {
				return nil, err
			}
			return applied(), nil
		})
	}
}
