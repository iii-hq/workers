package main

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/mail"
	"net/url"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"
)

type CloudActionRequest struct {
	MessageID string `json:"message_id"`
}
type CloudActionResult struct {
	Success bool `json:"success"`
}
type CloudReactionRequest struct {
	RecipientIdentityKeyHash string `json:"recipient_identity_key_hash,omitempty"`
	ChatID                   string `json:"chat_id"`
	MessageID                string `json:"message_id"`
	Emoji                    string `json:"emoji"`
}
type CloudLocationRequest struct {
	RecipientIdentityKeyHash string  `json:"recipient_identity_key_hash,omitempty"`
	ChatID                   string  `json:"chat_id"`
	Latitude                 float64 `json:"latitude"`
	Longitude                float64 `json:"longitude"`
	Name                     string  `json:"name,omitempty"`
	Address                  string  `json:"address,omitempty"`
	ReplyToMessageID         string  `json:"reply_to_message_id,omitempty"`
}
type CloudContactName struct {
	FormattedName string `json:"formatted_name"`
	FirstName     string `json:"first_name,omitempty"`
	LastName      string `json:"last_name,omitempty"`
	MiddleName    string `json:"middle_name,omitempty"`
	Suffix        string `json:"suffix,omitempty"`
	Prefix        string `json:"prefix,omitempty"`
}
type CloudContactAddress struct {
	Street      string `json:"street,omitempty"`
	City        string `json:"city,omitempty"`
	State       string `json:"state,omitempty"`
	Zip         string `json:"zip,omitempty"`
	Country     string `json:"country,omitempty"`
	CountryCode string `json:"country_code,omitempty"`
	Type        string `json:"type,omitempty"`
}
type CloudContactEmail struct {
	Email string `json:"email"`
	Type  string `json:"type,omitempty"`
}
type CloudContactPhone struct {
	Phone string `json:"phone,omitempty"`
	WAID  string `json:"wa_id,omitempty"`
	Type  string `json:"type,omitempty"`
}
type CloudContactURL struct {
	URL  string `json:"url"`
	Type string `json:"type,omitempty"`
}
type CloudContactOrg struct {
	Company    string `json:"company,omitempty"`
	Department string `json:"department,omitempty"`
	Title      string `json:"title,omitempty"`
}
type CloudContact struct {
	Name      CloudContactName      `json:"name"`
	Addresses []CloudContactAddress `json:"addresses,omitempty"`
	Birthday  string                `json:"birthday,omitempty"`
	Emails    []CloudContactEmail   `json:"emails,omitempty"`
	Org       *CloudContactOrg      `json:"org,omitempty"`
	Phones    []CloudContactPhone   `json:"phones,omitempty"`
	URLs      []CloudContactURL     `json:"urls,omitempty"`
}
type CloudContactsRequest struct {
	RecipientIdentityKeyHash string         `json:"recipient_identity_key_hash,omitempty"`
	ChatID                   string         `json:"chat_id"`
	Contacts                 []CloudContact `json:"contacts"`
	ReplyToMessageID         string         `json:"reply_to_message_id,omitempty"`
}

func cloudFieldText(field, value string, max int, required bool) error {
	if !utf8.ValidString(value) || utf8.RuneCountInString(value) > max || required && strings.TrimSpace(value) == "" || strings.IndexFunc(value, func(r rune) bool { return unicode.IsControl(r) && r != '\n' && r != '\t' }) >= 0 {
		return cloudError("invalid_request", fmt.Sprintf("%s must contain %sno more than %d Unicode characters without control characters", field, map[bool]string{true: "non-empty text and ", false: ""}[required], max))
	}
	return nil
}
func cloudMessageReference(id string) error {
	if id == "" || len(id) > 512 || strings.IndexFunc(id, unicode.IsSpace) >= 0 || strings.IndexFunc(id, unicode.IsControl) >= 0 {
		return cloudError("invalid_request", "message_id must contain 1..512 bytes without whitespace or control characters")
	}
	return nil
}
func cloudDisplayURL(raw string) error {
	u, err := url.Parse(raw)
	if err != nil || len(raw) > 2048 || (u.Scheme != "https" && u.Scheme != "http") || u.Hostname() == "" || u.User != nil || strings.IndexFunc(raw, unicode.IsSpace) >= 0 || strings.IndexFunc(raw, unicode.IsControl) >= 0 {
		return cloudError("invalid_request", "url must be an absolute HTTP(S) display URL without embedded credentials, at most 2048 bytes")
	}
	return nil
}
func cloudReactionPayload(req CloudReactionRequest) (map[string]any, error) {
	if err := cloudMessageReference(req.MessageID); err != nil {
		return nil, err
	}
	// Bound emoji sequences without assuming that a Unicode code point equals an
	// emoji: skin tones, flags, variation selectors and ZWJ sequences are valid.
	if len(req.Emoji) > 128 || !utf8.ValidString(req.Emoji) {
		return nil, cloudError("invalid_request", "emoji must be an emoji sequence of at most 128 UTF-8 bytes (worker limit), or empty to remove")
	}
	emojiBase := false
	for _, r := range req.Emoji {
		switch {
		case r >= 0x1F000 && r <= 0x1FAFF, r >= 0x2600 && r <= 0x27FF, r == 0x00A9, r == 0x00AE, r == 0x203C, r == 0x2049, r == 0x2122, r == 0x2139, r >= 0x2194 && r <= 0x21FF, r >= 0x2300 && r <= 0x23FF, r >= 0x2B00 && r <= 0x2BFF, r == 0x3030, r == 0x303D, r == 0x3297, r == 0x3299, r == 0x20E3:
			emojiBase = true
		case r == 0x200D, r == 0xFE0F, r == 0xFE0E, r >= '0' && r <= '9', r == '#', r == '*', r >= 0xE0020 && r <= 0xE007F:
		default:
			return nil, cloudError("invalid_request", "emoji must contain only an emoji sequence, or be empty to remove a reaction")
		}
	}
	if req.Emoji != "" && !emojiBase {
		return nil, cloudError("invalid_request", "emoji must contain an emoji")
	}
	body, err := cloudRecipient(req.ChatID)
	if err != nil {
		return nil, cloudError("invalid_request", err.Error())
	}
	if err := cloudApplyIdentity(body, req.RecipientIdentityKeyHash); err != nil {
		return nil, err
	}
	body["type"] = "reaction"
	body["reaction"] = map[string]any{"message_id": req.MessageID, "emoji": req.Emoji}
	return body, nil
}
func cloudLocationPayload(req CloudLocationRequest) (map[string]any, error) {
	if err := cloudFieldText("name", req.Name, 1024, false); err != nil {
		return nil, err
	}
	if err := cloudFieldText("address", req.Address, 1024, false); err != nil {
		return nil, err
	}
	body, err := cloudRecipient(req.ChatID)
	if err != nil {
		return nil, cloudError("invalid_request", err.Error())
	}
	location := map[string]any{"latitude": req.Latitude, "longitude": req.Longitude}
	if req.Name != "" {
		location["name"] = req.Name
	}
	if req.Address != "" {
		location["address"] = req.Address
	}
	body["type"] = "location"
	body["location"] = location
	if err := cloudApplyIdentity(body, req.RecipientIdentityKeyHash); err != nil {
		return nil, err
	}
	if err := cloudApplyReply(body, req.ReplyToMessageID); err != nil {
		return nil, err
	}
	return body, nil
}
func cloudContactsPayload(req CloudContactsRequest) (map[string]any, error) {
	raw, err := json.Marshal(req.Contacts)
	if err != nil || len(raw) > 65536 {
		return nil, cloudError("invalid_request", "contacts exceed the worker limit of 65536 JSON bytes")
	}
	for _, c := range req.Contacts {
		if err := cloudFieldText("formatted_name", c.Name.FormattedName, 1024, true); err != nil {
			return nil, err
		}
		for field, value := range map[string]string{"first_name": c.Name.FirstName, "last_name": c.Name.LastName, "middle_name": c.Name.MiddleName, "suffix": c.Name.Suffix, "prefix": c.Name.Prefix} {
			if err := cloudFieldText(field, value, 1024, false); err != nil {
				return nil, err
			}
		}
		if c.Birthday != "" {
			if _, err := time.Parse("2006-01-02", c.Birthday); err != nil {
				return nil, cloudError("invalid_request", "birthday must be a valid YYYY-MM-DD date")
			}
		}
		for _, e := range c.Emails {
			parsed, err := mail.ParseAddress(e.Email)
			if err != nil || parsed.Address != e.Email {
				return nil, cloudError("invalid_request", "contact email must be an email address")
			}
		}
		for _, u := range c.URLs {
			if err := cloudDisplayURL(u.URL); err != nil {
				return nil, err
			}
		}
		for _, p := range c.Phones {
			if p.WAID != "" {
				if _, err := normalizeCloudChatID(p.WAID); err != nil || strings.Contains(p.WAID, ".") {
					return nil, cloudError("invalid_request", "contact wa_id must be an international numeric WhatsApp user ID")
				}
			}
		}
	}
	body, err := cloudRecipient(req.ChatID)
	if err != nil {
		return nil, cloudError("invalid_request", err.Error())
	}
	body["type"] = "contacts"
	body["contacts"] = req.Contacts
	if err := cloudApplyIdentity(body, req.RecipientIdentityKeyHash); err != nil {
		return nil, err
	}
	if err := cloudApplyReply(body, req.ReplyToMessageID); err != nil {
		return nil, err
	}
	return body, nil
}

func registerCloudActions(r *Registry, client *cloudClient) {
	for _, op := range []string{"mark_read", "typing"} {
		r.Add("whatsapp::cloud::messages::"+op, func(ctx context.Context, input map[string]any) (any, error) {
			var req CloudActionRequest
			if err := decodeCloudRequest(input, &req); err != nil {
				return nil, cloudError("invalid_request", "Invalid read/typing request")
			}
			if err := cloudMessageReference(req.MessageID); err != nil {
				return nil, err
			}
			body := map[string]any{"messaging_product": "whatsapp", "status": "read", "message_id": req.MessageID}
			if op == "typing" {
				body["typing_indicator"] = map[string]any{"type": "text"}
			}
			config, err := r.Manager.ConfigContext(ctx)
			if err != nil {
				return nil, err
			}
			response, err := client.request(ctx, config, http.MethodPost, config.Cloud.PhoneNumberID+"/messages", nil, body)
			if err != nil {
				return nil, err
			}
			if response["success"] != true {
				return nil, cloudResponseError(http.MethodPost, "Cloud API returned no successful action acknowledgement")
			}
			return CloudActionResult{Success: true}, nil
		})
	}
	r.Add("whatsapp::cloud::messages::send_reaction", func(ctx context.Context, input map[string]any) (any, error) {
		var req CloudReactionRequest
		if err := decodeCloudRequest(input, &req); err != nil {
			return nil, cloudError("invalid_request", "Invalid reaction request")
		}
		body, err := cloudReactionPayload(req)
		if err != nil {
			return nil, err
		}
		return cloudSendAndRetain(ctx, r, client, req.ChatID, body, "reaction", req.Emoji)
	})
	r.Add("whatsapp::cloud::messages::send_location", func(ctx context.Context, input map[string]any) (any, error) {
		var req CloudLocationRequest
		if err := decodeCloudRequest(input, &req); err != nil {
			return nil, cloudError("invalid_request", "Invalid location request")
		}
		body, err := cloudLocationPayload(req)
		if err != nil {
			return nil, err
		}
		return cloudSendAndRetain(ctx, r, client, req.ChatID, body, "location", req.Name)
	})
	r.Add("whatsapp::cloud::messages::send_contacts", func(ctx context.Context, input map[string]any) (any, error) {
		var req CloudContactsRequest
		if err := decodeCloudRequest(input, &req); err != nil {
			return nil, cloudError("invalid_request", "Invalid contacts request")
		}
		body, err := cloudContactsPayload(req)
		if err != nil {
			return nil, err
		}
		return cloudSendAndRetain(ctx, r, client, req.ChatID, body, "contacts", req.Contacts[0].Name.FormattedName)
	})
}
