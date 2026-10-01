package main

import (
	"context"
	"crypto/hmac"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/hex"
	"encoding/json"
	"errors"
	"math"
	"strconv"
	"strings"
	"time"
	"unicode"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

const cloudWebhookMaxBody = 3 * 1024 * 1024

type cloudHTTPResponse struct {
	StatusCode int               `json:"status_code"`
	Headers    map[string]string `json:"headers"`
	Body       string            `json:"body"`
}

func cloudHTTPText(code int, body string) cloudHTTPResponse {
	return cloudHTTPResponse{code, map[string]string{"content-type": "text/plain; charset=utf-8"}, body}
}

type cloudVerifyRequest struct {
	QueryParams map[string]string `json:"query_params"`
}
type cloudReceiveRequest struct {
	Headers     map[string]string     `json:"headers"`
	RequestBody *iii.StreamChannelRef `json:"request_body"`
}

func registerCloudWebhooks(client *iii.Client, manager *Manager, engineURL string) error {
	metadata := iii.RegisterFunctionOptions{Metadata: json.RawMessage(`{"internal":true,"trace_hidden":true}`)}
	if err := client.RegisterFunction("whatsapp::cloud::webhook::verify", func(ctx context.Context, raw json.RawMessage) (any, error) {
		var req cloudVerifyRequest
		if err := json.Unmarshal(raw, &req); err != nil {
			return cloudHTTPText(400, "Invalid verification request"), nil
		}
		return verifyCloudWebhook(manager.Config().Cloud, req.QueryParams), nil
	}, metadata); err != nil {
		return err
	}
	if err := client.RegisterFunction("whatsapp::cloud::webhook::receive", func(ctx context.Context, raw json.RawMessage) (any, error) {
		var req cloudReceiveRequest
		if err := json.Unmarshal(raw, &req); err != nil {
			return cloudHTTPText(400, "Invalid webhook request"), nil
		}
		return receiveCloudWebhook(ctx, manager, engineURL, req), nil
	}, metadata); err != nil {
		return err
	}
	for _, route := range []struct{ method, function string }{{"GET", "whatsapp::cloud::webhook::verify"}, {"POST", "whatsapp::cloud::webhook::receive"}} {
		config, _ := json.Marshal(map[string]any{"api_path": cloudWebhookPath(), "http_method": route.method, "public_webhook": true})
		if err := client.RegisterTriggerNamespaced(ConfigID()+"-cloud-webhook-"+strings.ToLower(route.method), "http", route.function, Namespace(), config); err != nil {
			return err
		}
	}
	return nil
}
func verifyCloudWebhook(config CloudConfig, query map[string]string) cloudHTTPResponse {
	if !config.Enabled || strings.TrimSpace(config.VerifyToken) == "" {
		return cloudHTTPText(503, "Webhook verification is not configured")
	}
	if query["hub.mode"] != "subscribe" || len(query["hub.verify_token"]) > 16384 || subtle.ConstantTimeCompare([]byte(query["hub.verify_token"]), []byte(config.VerifyToken)) != 1 {
		return cloudHTTPText(403, "Invalid verification token or mode")
	}
	challenge := query["hub.challenge"]
	if challenge == "" || len(challenge) > 4096 {
		return cloudHTTPText(400, "Missing or invalid verification challenge")
	}
	return cloudHTTPText(200, challenge)
}
func cloudHeader(headers map[string]string, name string) string {
	for key, value := range headers {
		if strings.EqualFold(key, name) {
			return value
		}
	}
	return ""
}
func cloudSignatureBytes(signature string) ([]byte, error) {
	if !strings.HasPrefix(signature, "sha256=") || len(signature) != 71 {
		return nil, errors.New("invalid signature")
	}
	return hex.DecodeString(signature[7:])
}
func verifyCloudSignature(secret string, raw []byte, signature string) bool {
	expected, err := cloudSignatureBytes(signature)
	if err != nil {
		return false
	}
	mac := hmac.New(sha256.New, []byte(secret))
	_, _ = mac.Write(raw)
	return hmac.Equal(mac.Sum(nil), expected)
}
func receiveCloudWebhook(parent context.Context, manager *Manager, engineURL string, req cloudReceiveRequest) cloudHTTPResponse {
	config := manager.Config()
	if !config.Cloud.Enabled || strings.TrimSpace(config.Cloud.AppSecret) == "" {
		return cloudHTTPText(503, "Webhook delivery is not configured")
	}
	signature := cloudHeader(req.Headers, "x-hub-signature-256")
	if _, err := cloudSignatureBytes(signature); err != nil {
		return cloudHTTPText(403, "Invalid signature")
	}
	if size, err := strconv.ParseInt(cloudHeader(req.Headers, "content-length"), 10, 64); err == nil && size > cloudWebhookMaxBody {
		return cloudHTTPText(413, "Payload too large")
	}
	ref := req.RequestBody
	if ref == nil || ref.Direction != iii.ChannelRead || ref.ChannelID == "" || ref.AccessKey == "" {
		return cloudHTTPText(400, "Original request body is required")
	}
	ctx, cancel := context.WithTimeout(parent, 8*time.Second)
	defer cancel()
	reader := iii.OpenReader(engineURL, *ref)
	defer reader.Close()
	raw := []byte{}
	for {
		chunk, ok, err := reader.NextBinary(ctx)
		if err != nil {
			return cloudHTTPText(503, "Unable to read delivery; retry later")
		}
		if !ok {
			break
		}
		if len(chunk) > cloudWebhookMaxBody-len(raw) {
			return cloudHTTPText(413, "Payload too large")
		}
		raw = append(raw, chunk...)
	}
	if !verifyCloudSignature(config.Cloud.AppSecret, raw, signature) {
		return cloudHTTPText(403, "Invalid signature")
	}
	batch, err := parseCloudDelivery(config.Cloud, raw)
	if err != nil {
		return cloudHTTPText(400, "Invalid webhook delivery")
	}
	if len(batch.messages)+len(batch.receipts) == 0 {
		return cloudHTTPText(200, "Accepted")
	}
	done := make(chan error, 1)
	go func() {
		if manager.Config().Cloud != config.Cloud {
			done <- errors.New("Cloud configuration changed")
			return
		}
		inbox, err := manager.CloudInbox(config)
		if err == nil {
			_, err = inbox.RecordExternal(batch.messages, batch.receipts)
		}
		done <- err
	}()
	select {
	case err := <-done:
		if err != nil {
			return cloudHTTPText(503, "Unable to store delivery; retry later")
		}
		return cloudHTTPText(200, "Accepted")
	case <-ctx.Done():
		return cloudHTTPText(503, "Delivery timed out; retry later")
	}
}

type cloudDeliveryBatch struct {
	messages []MessageView
	receipts []ExternalReceipt
}

func cloudObject(value any) (map[string]any, error) {
	object, ok := value.(map[string]any)
	if !ok {
		return nil, errors.New("invalid webhook object")
	}
	return object, nil
}
func cloudRequiredString(value any) (string, error) {
	s, ok := value.(string)
	if !ok || s == "" {
		return "", errors.New("missing webhook field")
	}
	return s, nil
}
func cloudNested(object map[string]any, key string) map[string]any {
	nested, _ := object[key].(map[string]any)
	return nested
}
func cloudString(object map[string]any, key string) string {
	value, _ := object[key].(string)
	return value
}
func cloudMessageID(object map[string]any) (string, error) {
	id, err := cloudRequiredString(object["id"])
	if err != nil || len(id) > 512 || strings.IndexFunc(id, unicode.IsControl) >= 0 {
		return "", errors.New("invalid message ID")
	}
	return id, nil
}
func cloudExternalIdentity(object map[string]any, phone, userID string) (string, error) {
	raw := object[phone]
	if raw == nil {
		raw = object[userID]
	}
	id, err := cloudRequiredString(raw)
	if err != nil {
		return "", err
	}
	return normalizeCloudChatID(id)
}
func parseCloudDelivery(config CloudConfig, raw []byte) (cloudDeliveryBatch, error) {
	batch := cloudDeliveryBatch{messages: []MessageView{}, receipts: []ExternalReceipt{}}
	if len(raw) > cloudWebhookMaxBody {
		return batch, errors.New("payload exceeds limit")
	}
	var body map[string]any
	if err := json.Unmarshal(raw, &body); err != nil {
		return batch, err
	}
	if body["object"] != "whatsapp_business_account" {
		return batch, errors.New("unexpected webhook object")
	}
	entries, err := cloudArray(body["entry"])
	if err != nil {
		return batch, err
	}
	for _, rawEntry := range entries {
		entry, err := cloudObject(rawEntry)
		if err != nil {
			return batch, err
		}
		account, err := cloudRequiredString(entry["id"])
		if err != nil {
			return batch, err
		}
		if account != config.BusinessAccountID {
			continue
		}
		changes, err := cloudArray(entry["changes"])
		if err != nil {
			return batch, err
		}
		for _, rawChange := range changes {
			change, err := cloudObject(rawChange)
			if err != nil {
				return batch, err
			}
			field, err := cloudRequiredString(change["field"])
			if err != nil {
				return batch, err
			}
			if field != "messages" {
				continue
			}
			value, err := cloudObject(change["value"])
			if err != nil {
				return batch, err
			}
			phone, err := cloudRequiredString(cloudNested(value, "metadata")["phone_number_id"])
			if err != nil {
				return batch, err
			}
			if phone != config.PhoneNumberID {
				continue
			}
			if value["messaging_product"] != "whatsapp" {
				return batch, errors.New("unexpected messaging product")
			}
			names := map[string]string{}
			if rawContacts, exists := value["contacts"]; exists {
				contacts, err := cloudArray(rawContacts)
				if err != nil {
					return batch, err
				}
				for _, rawContact := range contacts {
					contact, err := cloudObject(rawContact)
					if err != nil {
						return batch, err
					}
					name, ok := cloudNested(contact, "profile")["name"].(string)
					if !ok {
						continue
					}
					for _, field := range []string{"wa_id", "user_id"} {
						if id, err := normalizeCloudChatID(cloudString(contact, field)); err == nil {
							names[id] = clipInboxText(name, 512)
						}
					}
				}
			}
			if rawMessages, exists := value["messages"]; exists {
				messages, err := cloudArray(rawMessages)
				if err != nil {
					return batch, err
				}
				for _, rawMessage := range messages {
					if len(batch.messages)+len(batch.receipts) >= 1000 {
						return batch, errors.New("delivery exceeds item limit")
					}
					message, err := cloudObject(rawMessage)
					if err != nil {
						return batch, err
					}
					sender, err := cloudExternalIdentity(message, "from", "from_user_id")
					if err != nil {
						return batch, err
					}
					id, err := cloudMessageID(message)
					if err != nil {
						return batch, err
					}
					seconds, err := strconv.ParseInt(cloudString(message, "timestamp"), 10, 64)
					if err != nil || seconds < 0 || seconds > math.MaxInt64/1000 {
						return batch, errors.New("invalid message timestamp")
					}
					kind, text, err := cloudMessagePreview(message)
					if err != nil {
						return batch, err
					}
					chat, _ := cloudStorageChatID(sender)
					view := MessageView{MessageID: id, ChatID: chat, SenderID: sender, TimestampMS: seconds * 1000, Kind: kind, Text: text, Source: "live"}
					if name, ok := names[sender]; ok {
						view.SenderName = &name
					}
					batch.messages = append(batch.messages, view)
				}
			}
			if rawStatuses, exists := value["statuses"]; exists {
				statuses, err := cloudArray(rawStatuses)
				if err != nil {
					return batch, err
				}
				for _, rawStatus := range statuses {
					status, err := cloudObject(rawStatus)
					if err != nil {
						return batch, err
					}
					state, err := cloudRequiredString(status["status"])
					if err != nil {
						return batch, err
					}
					if externalStatusRank(state) == 0 {
						continue
					}
					if len(batch.messages)+len(batch.receipts) >= 1000 {
						return batch, errors.New("delivery exceeds item limit")
					}
					recipient, err := cloudExternalIdentity(status, "recipient_id", "recipient_user_id")
					if err != nil {
						return batch, err
					}
					id, err := cloudMessageID(status)
					if err != nil {
						return batch, err
					}
					chat, _ := cloudStorageChatID(recipient)
					batch.receipts = append(batch.receipts, ExternalReceipt{MessageID: id, ChatID: chat, Status: state})
				}
			}
		}
	}
	return batch, nil
}
func cloudMessagePreview(message map[string]any) (string, string, error) {
	kind, err := cloudRequiredString(message["type"])
	if err != nil {
		return "", "", err
	}
	text := ""
	switch kind {
	case "text":
		var ok bool
		text, ok = cloudNested(message, "text")["body"].(string)
		if !ok {
			return "", "", errors.New("missing message text")
		}
	case "image", "video", "audio", "document", "sticker":
		media, err := cloudObject(message[kind])
		if err != nil {
			return "", "", err
		}
		if caption, exists := media["caption"]; exists {
			var ok bool
			text, ok = caption.(string)
			if !ok {
				return "", "", errors.New("invalid caption")
			}
		}
	case "button":
		kind = "text"
		text = cloudString(cloudNested(message, "button"), "text")
		if text == "" {
			text = "Button reply"
		}
	case "interactive":
		kind = "text"
		interactive := cloudNested(message, "interactive")
		text = cloudString(cloudNested(interactive, "button_reply"), "title")
		if text == "" {
			text = cloudString(cloudNested(interactive, "list_reply"), "title")
		}
		if text == "" {
			text = "Interactive reply"
		}
	case "reaction":
		text = cloudString(cloudNested(message, "reaction"), "emoji")
		if text == "" {
			text = "Reaction removed"
		}
	case "location":
		text = cloudString(cloudNested(message, "location"), "name")
		if text == "" {
			text = "Location"
		}
	case "contacts":
		kind = "contact"
		text = "Shared contact"
	case "system":
		text = "Account update"
	default:
		kind = "unknown"
		text = "Unsupported message"
	}
	return kind, clipInboxText(text, 16384), nil
}
