package main

import (
	"context"
	"encoding/json"
	"strings"
)

type CloudInteractiveHeader struct {
	Type    string `json:"type"`
	Text    string `json:"text,omitempty"`
	MediaID string `json:"media_id,omitempty"`
}
type CloudInteractiveBase struct {
	ChatID                   string                  `json:"chat_id"`
	Text                     string                  `json:"text"`
	Header                   *CloudInteractiveHeader `json:"header,omitempty"`
	Footer                   string                  `json:"footer,omitempty"`
	ReplyToMessageID         string                  `json:"reply_to_message_id,omitempty"`
	RecipientIdentityKeyHash string                  `json:"recipient_identity_key_hash,omitempty"`
}
type CloudReplyButton struct {
	ID    string `json:"id"`
	Title string `json:"title"`
}
type CloudButtonsRequest struct {
	CloudInteractiveBase
	Buttons []CloudReplyButton `json:"buttons"`
}
type CloudListRow struct {
	ID          string `json:"id"`
	Title       string `json:"title"`
	Description string `json:"description,omitempty"`
}
type CloudListSection struct {
	Title string         `json:"title"`
	Rows  []CloudListRow `json:"rows"`
}
type CloudListRequest struct {
	CloudInteractiveBase
	Button   string             `json:"button"`
	Sections []CloudListSection `json:"sections"`
}
type CloudCTARequest struct {
	CloudInteractiveBase
	DisplayText string `json:"display_text"`
	URL         string `json:"url"`
}
type CloudCarouselCard struct {
	Header      CloudInteractiveHeader `json:"header"`
	Text        string                 `json:"text,omitempty"`
	Buttons     []CloudReplyButton     `json:"buttons,omitempty"`
	DisplayText string                 `json:"display_text,omitempty"`
	URL         string                 `json:"url,omitempty"`
}
type CloudCarouselRequest struct {
	CloudInteractiveBase
	Cards []CloudCarouselCard `json:"cards"`
}
type CloudFlowRequest struct {
	CloudInteractiveBase
	FlowID     string         `json:"flow_id,omitempty"`
	FlowName   string         `json:"flow_name,omitempty"`
	FlowCTA    string         `json:"flow_cta"`
	FlowToken  string         `json:"flow_token,omitempty"`
	Mode       string         `json:"mode,omitempty"`
	FlowAction string         `json:"flow_action,omitempty"`
	Screen     string         `json:"screen,omitempty"`
	Data       map[string]any `json:"data,omitempty"`
}

func cloudHeaderPayload(header *CloudInteractiveHeader) (map[string]any, error) {
	if header == nil {
		return nil, nil
	}
	result := map[string]any{"type": header.Type}
	if header.Type == "text" {
		if header.MediaID != "" {
			return nil, cloudError("invalid_request", "a text header cannot contain media_id")
		}
		if err := cloudFieldText("header.text", header.Text, 60, true); err != nil {
			return nil, err
		}
		result["text"] = header.Text
	} else {
		if header.Text != "" || !cloudDocumentMediaID(header.MediaID) {
			return nil, cloudError("invalid_request", "media headers require a numeric media_id and cannot contain text or hosted links")
		}
		result[header.Type] = map[string]any{"id": header.MediaID}
	}
	return result, nil
}
func cloudInteractivePayload(base CloudInteractiveBase, kind string, maxBody int, action map[string]any) (map[string]any, error) {
	// Single-product messages are the only kind with an optional body.
	if err := cloudFieldText("text", base.Text, maxBody, kind != "product"); err != nil {
		return nil, err
	}
	if err := cloudFieldText("footer", base.Footer, 60, false); err != nil {
		return nil, err
	}
	header, err := cloudHeaderPayload(base.Header)
	if err != nil {
		return nil, err
	}
	body, err := cloudRecipient(base.ChatID)
	if err != nil {
		return nil, cloudError("invalid_request", err.Error())
	}
	interactive := map[string]any{"type": kind, "action": action}
	if base.Text != "" {
		interactive["body"] = map[string]any{"text": base.Text}
	}
	if header != nil {
		interactive["header"] = header
	}
	if base.Footer != "" {
		interactive["footer"] = map[string]any{"text": base.Footer}
	}
	body["type"] = "interactive"
	body["interactive"] = interactive
	if err := cloudApplyReply(body, base.ReplyToMessageID); err != nil {
		return nil, err
	}
	if err := cloudApplyIdentity(body, base.RecipientIdentityKeyHash); err != nil {
		return nil, err
	}
	return body, nil
}
func cloudButtonsPayload(buttons []CloudReplyButton, kind string) ([]map[string]any, error) {
	if len(buttons) < 1 || len(buttons) > 3 {
		return nil, cloudError("invalid_request", "buttons must contain 1..3 entries")
	}
	ids, titles := map[string]bool{}, map[string]bool{}
	out := make([]map[string]any, 0, len(buttons))
	for _, b := range buttons {
		if err := cloudFieldText("button.id", b.ID, 256, true); err != nil {
			return nil, err
		}
		if strings.TrimSpace(b.ID) != b.ID {
			return nil, cloudError("invalid_request", "button.id must not have leading or trailing whitespace")
		}
		if err := cloudFieldText("button.title", b.Title, 20, true); err != nil {
			return nil, err
		}
		if ids[b.ID] || titles[b.Title] {
			return nil, cloudError("invalid_request", "button IDs and titles must be unique")
		}
		ids[b.ID] = true
		titles[b.Title] = true
		out = append(out, map[string]any{"type": kind, kind: map[string]any{"id": b.ID, "title": b.Title}})
	}
	return out, nil
}
func cloudListPayload(req CloudListRequest) (map[string]any, error) {
	if err := cloudFieldText("button", req.Button, 20, true); err != nil {
		return nil, err
	}
	count := 0
	ids := map[string]bool{}
	for _, section := range req.Sections {
		if err := cloudFieldText("section.title", section.Title, 24, true); err != nil {
			return nil, err
		}
		for _, row := range section.Rows {
			count++
			if err := cloudFieldText("row.id", row.ID, 200, true); err != nil {
				return nil, err
			}
			if err := cloudFieldText("row.title", row.Title, 24, true); err != nil {
				return nil, err
			}
			if err := cloudFieldText("row.description", row.Description, 72, false); err != nil {
				return nil, err
			}
			if ids[row.ID] {
				return nil, cloudError("invalid_request", "row IDs must be unique across all sections")
			}
			ids[row.ID] = true
		}
	}
	if count > 10 {
		return nil, cloudError("invalid_request", "lists support at most 10 rows across all sections")
	}
	return cloudInteractivePayload(req.CloudInteractiveBase, "list", 4096, map[string]any{"button": req.Button, "sections": req.Sections})
}
func cloudCTAPayload(label, rawURL string) (map[string]any, error) {
	if err := cloudFieldText("display_text", label, 20, true); err != nil {
		return nil, err
	}
	if err := cloudDisplayURL(rawURL); err != nil {
		return nil, err
	}
	return map[string]any{"name": "cta_url", "parameters": map[string]any{"display_text": label, "url": rawURL}}, nil
}
func cloudCarouselPayload(req CloudCarouselRequest) (map[string]any, error) {
	cards := make([]map[string]any, 0, len(req.Cards))
	buttonCount := -1
	var useURL bool
	for n, card := range req.Cards {
		header, err := cloudHeaderPayload(&card.Header)
		if err != nil {
			return nil, err
		}
		if err := cloudFieldText("card.text", card.Text, 160, false); err != nil {
			return nil, err
		}
		if strings.Count(card.Text, "\n") > 2 {
			return nil, cloudError("invalid_request", "card text supports at most two line breaks")
		}
		isURL := card.URL != "" || card.DisplayText != ""
		if n == 0 {
			useURL = isURL
			buttonCount = len(card.Buttons)
		} else if isURL != useURL || len(card.Buttons) != buttonCount {
			return nil, cloudError("invalid_request", "all carousel cards must use the same button type and count")
		}
		var action map[string]any
		if isURL {
			if len(card.Buttons) > 0 {
				return nil, cloudError("invalid_request", "a carousel card must use either a URL button or quick replies")
			}
			action, err = cloudCTAPayload(card.DisplayText, card.URL)
		} else {
			var buttons []map[string]any
			buttons, err = cloudButtonsPayload(card.Buttons, "quick_reply")
			action = map[string]any{"buttons": buttons}
		}
		if err != nil {
			return nil, err
		}
		// Meta's current carousel examples use cta_url for both action variants.
		item := map[string]any{"card_index": n, "type": "cta_url", "header": header, "action": action}
		if card.Text != "" {
			item["body"] = map[string]any{"text": card.Text}
		}
		cards = append(cards, item)
	}
	return cloudInteractivePayload(req.CloudInteractiveBase, "carousel", 1024, map[string]any{"cards": cards})
}
func cloudFlowPayload(req CloudFlowRequest) (map[string]any, error) {
	if (req.FlowID == "") == (req.FlowName == "") {
		return nil, cloudError("invalid_request", "exactly one of flow_id or flow_name is required")
	}
	if err := cloudFieldText("flow_name", req.FlowName, 255, false); err != nil {
		return nil, err
	}
	if err := cloudFieldText("flow_cta", req.FlowCTA, 30, true); err != nil {
		return nil, err
	}
	if err := cloudFieldText("flow_token", req.FlowToken, 4096, false); err != nil {
		return nil, err
	}
	if req.Mode == "" {
		req.Mode = "published"
	}
	if req.FlowAction == "" {
		req.FlowAction = "navigate"
	}
	params := map[string]any{"flow_message_version": "3", "flow_cta": req.FlowCTA, "mode": req.Mode, "flow_action": req.FlowAction}
	if req.FlowID != "" {
		params["flow_id"] = req.FlowID
	} else {
		params["flow_name"] = req.FlowName
	}
	if req.FlowToken != "" {
		params["flow_token"] = req.FlowToken
	}
	if req.FlowAction == "navigate" {
		if err := cloudFieldText("screen", req.Screen, 256, true); err != nil {
			return nil, err
		}
		payload := map[string]any{"screen": req.Screen}
		if req.Data != nil {
			raw, err := json.Marshal(req.Data)
			if err != nil || len(raw) > 16384 {
				return nil, cloudError("invalid_request", "data must be a JSON object of at most 16384 bytes (worker limit)")
			}
			payload["data"] = req.Data
		}
		params["flow_action_payload"] = payload
	} else if req.Screen != "" || req.Data != nil {
		return nil, cloudError("invalid_request", "screen and data are supported only for navigate flows; data_exchange uses the configured Flow endpoint")
	}
	return cloudInteractivePayload(req.CloudInteractiveBase, "flow", 1024, map[string]any{"name": "flow", "parameters": params})
}

func (b CloudInteractiveBase) interactiveBase() CloudInteractiveBase { return b }

// addCloudInteractive registers a send whose request embeds CloudInteractiveBase.
func addCloudInteractive[T interface{ interactiveBase() CloudInteractiveBase }](r *Registry, client *cloudClient, name string, build func(T) (map[string]any, error)) {
	r.Add("whatsapp::cloud::messages::"+name, func(ctx context.Context, input map[string]any) (any, error) {
		var req T
		if err := decodeCloudRequest(input, &req); err != nil {
			return nil, cloudError("invalid_request", "Invalid "+name+" request")
		}
		body, err := build(req)
		if err != nil {
			return nil, err
		}
		base := req.interactiveBase()
		return cloudSendAndRetain(ctx, r, client, base.ChatID, body, "interactive", base.Text)
	})
}

func registerCloudInteractive(r *Registry, client *cloudClient) {
	addCloudInteractive(r, client, "send_list", cloudListPayload)
	addCloudInteractive(r, client, "send_buttons", func(req CloudButtonsRequest) (map[string]any, error) {
		buttons, err := cloudButtonsPayload(req.Buttons, "reply")
		if err != nil {
			return nil, err
		}
		return cloudInteractivePayload(req.CloudInteractiveBase, "button", 1024, map[string]any{"buttons": buttons})
	})
	addCloudInteractive(r, client, "send_cta_url", func(req CloudCTARequest) (map[string]any, error) {
		action, err := cloudCTAPayload(req.DisplayText, req.URL)
		if err != nil {
			return nil, err
		}
		return cloudInteractivePayload(req.CloudInteractiveBase, "cta_url", 1024, action)
	})
	addCloudInteractive(r, client, "request_location", func(req CloudInteractiveBase) (map[string]any, error) {
		return cloudInteractivePayload(req, "location_request_message", 1024, map[string]any{"name": "send_location"})
	})
	addCloudInteractive(r, client, "send_carousel", cloudCarouselPayload)
	addCloudInteractive(r, client, "send_flow", cloudFlowPayload)
}
