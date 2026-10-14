package main

import (
	"context"
	"encoding/json"
	"errors"
	"sort"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/encoding/protojson"
	"google.golang.org/protobuf/proto"
)

func nativeNullable(s string) any {
	if s == "" {
		return nil
	}
	return s
}
func nativeNewsletter(raw string) (types.JID, error) {
	jid, err := parseJID(raw)
	if err != nil {
		return jid, err
	}
	if jid.Server != types.NewsletterServer {
		return jid, errors.New("newsletter_id must end in @newsletter")
	}
	return jid, nil
}
func nativeNewsletterMetadata(v *types.NewsletterMetadata) map[string]any {
	meta := v.ThreadMeta
	out := map[string]any{"newsletter_id": v.ID.String(), "name": meta.Name.Text, "description": nativeNullable(meta.Description.Text), "subscriber_count": meta.SubscriberCount, "verification": string(meta.VerificationState), "state": string(v.State.Type), "picture_url": nil, "preview_url": nativeNullable(meta.Preview.URL), "invite_code": nativeNullable(meta.InviteCode), "role": nil, "creation_time": nil, "muted": nil, "follower_activity_muted": nil}
	if meta.Picture != nil {
		out["picture_url"] = nativeNullable(meta.Picture.URL)
	}
	if !meta.CreationTime.IsZero() {
		out["creation_time"] = meta.CreationTime.Unix()
	}
	if v.ViewerMeta != nil {
		out["role"] = nativeNullable(string(v.ViewerMeta.Role))
		if v.ViewerMeta.Mute != "" {
			out["muted"] = v.ViewerMeta.Mute == types.NewsletterMuteOn
		}
	}
	return out
}
func nativeNewsletterMessage(v *types.NewsletterMessage) (map[string]any, error) {
	var content any
	if v.Message != nil {
		data, err := (protojson.MarshalOptions{UseProtoNames: true}).Marshal(v.Message)
		if err != nil {
			return nil, err
		}
		if err = json.Unmarshal(data, &content); err != nil {
			return nil, err
		}
	}
	emojis := make([]string, 0, len(v.ReactionCounts))
	for emoji := range v.ReactionCounts {
		emojis = append(emojis, emoji)
	}
	sort.Strings(emojis)
	var reactions []map[string]any
	if v.ReactionCounts != nil {
		reactions = make([]map[string]any, 0, len(emojis))
	}
	for _, emoji := range emojis {
		reactions = append(reactions, map[string]any{"emoji": emoji, "count": v.ReactionCounts[emoji]})
	}
	var timestamp, views any
	if !v.Timestamp.IsZero() {
		timestamp = v.Timestamp.Unix()
	}
	// whatsmeow uses zero for both a missing view counter and a known zero.
	if v.ViewsCount > 0 {
		views = v.ViewsCount
	}
	return map[string]any{"message_id": v.MessageID, "server_id": v.MessageServerID, "timestamp": timestamp, "message_type": v.Type, "is_sender": nil, "content": content, "reactions": reactions, "votes": nil, "forwards_count": nil, "views_count": views, "responses_count": nil, "original_timestamp": nil, "last_edit_timestamp_ms": nil, "media_type": nil, "poll_type": nil, "admin_profile": nil}, nil
}

func registerNativeNewsletters(r *Registry) {
	r.Add("whatsapp::newsletters::list", func(ctx context.Context, args map[string]any) (any, error) {
		client, err := r.Manager.ClientContext(ctx, stringArg(args, "session_id"))
		if err != nil {
			return nil, err
		}
		items, err := client.GetSubscribedNewsletters(ctx)
		if err != nil {
			return nil, err
		}
		out := make([]map[string]any, 0, len(items))
		for _, item := range items {
			out = append(out, nativeNewsletterMetadata(item))
		}
		return out, nil
	})
	r.Add("whatsapp::newsletters::create", func(ctx context.Context, args map[string]any) (any, error) {
		name, description := stringArg(args, "name"), stringArg(args, "description")
		if err := nativeText(name, "name", 100, false); err != nil {
			return nil, err
		}
		if err := nativeText(description, "description", 2048, true); err != nil {
			return nil, err
		}
		client, err := r.Manager.ClientContext(ctx, stringArg(args, "session_id"))
		if err != nil {
			return nil, err
		}
		meta, err := client.CreateNewsletter(ctx, whatsmeow.CreateNewsletterParams{Name: name, Description: description})
		if err != nil {
			return nil, err
		}
		return nativeNewsletterMetadata(meta), nil
	})
	r.Add("whatsapp::newsletters::get_by_invite", func(ctx context.Context, args map[string]any) (any, error) {
		code := stringArg(args, "invite_code")
		if err := nativeText(code, "invite_code", 256, false); err != nil {
			return nil, err
		}
		client, err := r.Manager.ClientContext(ctx, stringArg(args, "session_id"))
		if err != nil {
			return nil, err
		}
		meta, err := client.GetNewsletterInfoWithInvite(ctx, code)
		if err != nil {
			return nil, err
		}
		return nativeNewsletterMetadata(meta), nil
	})
	for _, action := range []string{"get", "join", "leave", "mute_updates", "subscribe_updates", "react", "edit", "revoke", "messages"} {
		r.Add("whatsapp::newsletters::"+action, func(ctx context.Context, args map[string]any) (any, error) {
			jid, err := nativeNewsletter(stringArg(args, "newsletter_id"))
			if err != nil {
				return nil, err
			}
			client, err := r.Manager.ClientContext(ctx, stringArg(args, "session_id"))
			if err != nil {
				return nil, err
			}
			switch action {
			case "get", "join":
				if action == "join" {
					if err = client.FollowNewsletter(ctx, jid); err != nil {
						return nil, err
					}
				}
				meta, err := client.GetNewsletterInfo(ctx, jid)
				if err != nil {
					if action == "join" {
						return nil, errors.New("channel was followed but metadata could not be loaded; do not retry the follow automatically")
					}
					return nil, err
				}
				return nativeNewsletterMetadata(meta), nil
			case "leave":
				if err = client.UnfollowNewsletter(ctx, jid); err != nil {
					return nil, err
				}
				return applied(), nil
			case "mute_updates":
				if err = client.NewsletterToggleMute(ctx, jid, boolArg(args, "muted")); err != nil {
					return nil, err
				}
				return applied(), nil
			case "subscribe_updates":
				duration, err := client.NewsletterSubscribeLiveUpdates(ctx, jid)
				if err != nil {
					return nil, err
				}
				return map[string]any{"duration_seconds": int64(duration.Seconds())}, nil
			case "react":
				serverID := intArg(args, "server_id", 0)
				if serverID <= 0 {
					return nil, errors.New("server_id must be positive")
				}
				emoji := stringArg(args, "emoji")
				if err = nativeText(emoji, "emoji", 64, true); err != nil {
					return nil, err
				}
				id := client.GenerateMessageID()
				if err = client.NewsletterSendReaction(ctx, jid, serverID, emoji, id); err != nil {
					return nil, err
				}
				return map[string]any{"stanza_id": id}, nil
			case "edit", "revoke":
				id := stringArg(args, "message_id")
				if err = nativeMessageID(id); err != nil {
					return nil, err
				}
				var payload *waE2E.Message
				if action == "edit" {
					text := stringArg(args, "text")
					if err = nativeText(text, "text", 65536, false); err != nil {
						return nil, err
					}
					payload = client.BuildEdit(jid, id, &waE2E.Message{Conversation: proto.String(text)})
				} else {
					payload = client.BuildRevoke(jid, types.EmptyJID, id)
				}
				if _, err = client.SendMessage(ctx, jid, payload, whatsmeow.SendRequestExtra{ID: id}); err != nil {
					return nil, err
				}
				return applied(), nil
			case "messages":
				count := intArg(args, "count", 0)
				if count < 1 || count > 100 {
					return nil, errors.New("count must be between 1 and 100")
				}
				before := intArg(args, "before_server_id", 0)
				if before < 0 {
					return nil, errors.New("before_server_id must be nonnegative")
				}
				items, err := client.GetNewsletterMessages(ctx, jid, &whatsmeow.GetNewsletterMessagesParams{Count: count, Before: before})
				if err != nil {
					return nil, err
				}
				out := make([]map[string]any, 0, len(items))
				for _, item := range items {
					mapped, err := nativeNewsletterMessage(item)
					if err != nil {
						return nil, err
					}
					out = append(out, mapped)
				}
				return out, nil
			}
			return nil, errors.New("unknown newsletter operation")
		})
	}
}
