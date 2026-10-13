package main

import (
	"context"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"net/url"
	"regexp"
	"slices"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"
)

var cloudGroupIDPattern = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9_+/=-]{0,255}$`)

func normalizeCloudConversationID(raw string) (string, error) {
	if strings.HasPrefix(raw, "group:") {
		if !cloudGroupIDPattern.MatchString(strings.TrimPrefix(raw, "group:")) {
			return "", cloudError("invalid_request", "Invalid Cloud group identifier")
		}
		return raw, nil
	}
	return normalizeCloudChatID(raw)
}

func cloudApplyReply(body map[string]any, messageID string) error {
	if messageID == "" {
		return nil
	}
	if len(messageID) > 512 || !utf8.ValidString(messageID) || strings.TrimSpace(messageID) != messageID || strings.IndexFunc(messageID, unicode.IsControl) >= 0 {
		return cloudError("invalid_request", "reply_to_message_id must be a valid Meta message ID of at most 512 bytes")
	}
	body["context"] = map[string]any{"message_id": messageID}
	return nil
}

func cloudApplyIdentity(body map[string]any, identityHash string) error {
	if identityHash == "" {
		return nil
	}
	if len(identityHash) > 512 || !utf8.ValidString(identityHash) || strings.IndexFunc(identityHash, unicode.IsSpace) >= 0 || strings.IndexFunc(identityHash, unicode.IsControl) >= 0 {
		return cloudError("invalid_request", "recipient_identity_key_hash must be a single non-whitespace value of at most 512 bytes")
	}
	body["recipient_identity_key_hash"] = identityHash
	return nil
}

// Prepare durable storage before dispatch; a successful POST is never replayed
// because archival failed after Meta accepted it.
func cloudSendAndRetain(ctx context.Context, r *Registry, client *cloudClient, chat string, body map[string]any, kind, preview string) (CloudSentMessage, error) {
	return cloudSendAndRetainAt(ctx, r, client, chat, body, kind, preview, "messages")
}
func cloudSendAndRetainAt(ctx context.Context, r *Registry, client *cloudClient, chat string, body map[string]any, kind, preview, endpoint string) (CloudSentMessage, error) {
	config, err := r.Manager.ConfigContext(ctx)
	if err != nil {
		return CloudSentMessage{}, err
	}
	if err := cloudRequestConfig(config); err != nil {
		return CloudSentMessage{}, err
	}
	inbox, err := r.Manager.CloudInboxContext(ctx, config)
	if err != nil {
		return CloudSentMessage{}, cloudError("cloud_storage_error", "Could not prepare Cloud API message archive")
	}
	requestedAt := time.Now()
	sent, err := client.sendAt(ctx, config, chat, endpoint, body)
	if err != nil {
		return sent, err
	}
	if err = retainCloudSendContext(ctx, inbox, sent, kind, preview, requestedAt); err != nil {
		return sent, cloudError("cloud_archive_failed", "Cloud API accepted message "+sent.MessageID+", but local archival failed; do not resend automatically")
	}
	return sent, nil
}

// Results may contain dynamic Graph fields, but credentials and authenticated
// pagination URLs are never part of the worker's public contracts.
func cloudSanitizeResult(config Config, value any) any {
	return cloudSanitizeValue(value, cloudCredentialVariants(config))
}

func cloudCredentialVariants(config Config) []string {
	variants := []string{}
	for _, secret := range []string{config.Cloud.AccessToken, config.Cloud.AppSecret, config.Cloud.VerifyToken, config.Cloud.FlowPrivateKey} {
		if secret == "" {
			continue
		}
		for _, encoded := range []string{secret, url.QueryEscape(secret), url.PathEscape(secret), base64.StdEncoding.EncodeToString([]byte(secret)), base64.RawStdEncoding.EncodeToString([]byte(secret)), base64.URLEncoding.EncodeToString([]byte(secret)), base64.RawURLEncoding.EncodeToString([]byte(secret)), hex.EncodeToString([]byte(secret))} {
			if !slices.Contains(variants, encoded) {
				variants = append(variants, encoded)
			}
		}
	}
	// Replace longer encodings before their unpadded prefixes.
	slices.SortFunc(variants, func(a, b string) int { return len(b) - len(a) })
	return variants
}

func cloudCredentialKey(key string) bool {
	key = strings.ToLower(strings.NewReplacer("_", "", "-", "", ".", "").Replace(key))
	switch key {
	case "accesstoken", "systemuseraccesstoken", "appsecret", "appsecretproof", "verifytoken", "authorization", "pin", "password", "privatekey", "businessprivatekey", "flowprivatekey", "clientsecret", "refreshtoken", "bearertoken", "sipuserpassword", "sippassword", "sipauthpassword", "sipusername", "sipauthusername":
		return true
	}
	return false
}

func cloudSanitizeValue(value any, variants []string) any {
	switch v := value.(type) {
	case map[string]any:
		out := make(map[string]any, len(v))
		for k, x := range v {
			key := strings.ToLower(k)
			if cloudCredentialKey(key) {
				continue
			}
			if key == "next" || key == "previous" {
				if link, ok := x.(string); ok {
					if u, err := url.Parse(link); err == nil && u.Host != "" && (u.Scheme == "http" || u.Scheme == "https") {
						continue
					}
				}
			}
			cleanKey := cloudSanitizeValue(k, variants).(string)
			out[cleanKey] = cloudSanitizeValue(x, variants)
		}
		return out
	case []any:
		out := make([]any, len(v))
		for i, x := range v {
			out[i] = cloudSanitizeValue(x, variants)
		}
		return out
	case []map[string]any:
		out := make([]any, len(v))
		for i, x := range v {
			out[i] = cloudSanitizeValue(x, variants)
		}
		return out
	case []string:
		out := make([]any, len(v))
		for i, x := range v {
			out[i] = cloudSanitizeValue(x, variants)
		}
		return out
	case *string:
		if v == nil {
			return nil
		}
		return cloudSanitizeValue(*v, variants)
	case json.RawMessage:
		var decoded any
		if json.Unmarshal(v, &decoded) != nil {
			return nil
		}
		return cloudSanitizeValue(decoded, variants)
	case string:
		for _, secret := range variants {
			// A short user-chosen verify token can occur incidentally inside an
			// opaque ID or ordinary text. Match the full scalar in that case.
			if len(secret) >= 8 {
				v = strings.ReplaceAll(v, secret, "[redacted]")
			} else if v == secret {
				v = "[redacted]"
			}
		}
		if u, err := url.Parse(v); err == nil && u.IsAbs() {
			changed := u.User != nil
			if u.User != nil {
				u.User = nil
			}
			query := u.Query()
			for key := range query {
				if cloudCredentialKey(key) {
					query.Del(key)
					changed = true
				}
			}
			if changed {
				u.RawQuery = query.Encode()
				v = u.String()
			}
		}
		return v
	default:
		return value
	}
}
