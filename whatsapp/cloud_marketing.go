package main

import (
	"context"
	"strings"
)

type CloudMarketingTemplateRequest struct {
	CloudTemplateRequest
	ProductPolicy          string `json:"product_policy,omitempty"`
	MessageActivitySharing *bool  `json:"message_activity_sharing,omitempty"`
}
type CloudDirectSendRequest struct {
	CloudInteractiveBase
	Category     string             `json:"category"`
	Type         string             `json:"type"`
	TTLSeconds   *int               `json:"ttl_seconds,omitempty"`
	TemplateName string             `json:"template_name,omitempty"`
	Buttons      []CloudReplyButton `json:"buttons,omitempty"`
	DisplayText  string             `json:"display_text,omitempty"`
	URL          string             `json:"url,omitempty"`
}

func cloudMarketingPayload(req CloudMarketingTemplateRequest) (map[string]any, error) {
	if req.ProductPolicy == "" {
		req.ProductPolicy = "STRICT"
	}
	body, err := cloudTemplatePayload(req.CloudTemplateRequest)
	if err != nil {
		return nil, cloudError("invalid_request", "Invalid marketing template fields")
	}
	body["product_policy"] = req.ProductPolicy
	if req.MessageActivitySharing != nil {
		body["message_activity_sharing"] = *req.MessageActivitySharing
	}
	return body, nil
}
func cloudDirectPayload(req CloudDirectSendRequest) (map[string]any, error) {
	chat, err := normalizeCloudChatID(req.ChatID)
	if err != nil {
		return nil, cloudError("invalid_request", err.Error())
	}
	if req.Category == "authentication" && !strings.HasPrefix(chat, "+") {
		return nil, cloudError("invalid_request", "authentication-category Direct Send requires a phone number; BSUID recipients are not supported")
	}
	if req.TTLSeconds != nil && (req.Category == "service" || req.Category == "authentication" && *req.TTLSeconds > 900) {
		return nil, cloudError("invalid_request", "ttl_seconds is available only for utility (30..43200) and authentication (30..900)")
	}
	if req.TemplateName != "" && (req.Category != "utility" || !cloudTemplateNamePattern.MatchString(req.TemplateName)) {
		return nil, cloudError("invalid_request", "template_name is a restricted utility-only Direct Send feature and must contain 1..512 lowercase letters, digits or underscores")
	}
	var body map[string]any
	switch req.Type {
	case "text":
		if req.Header != nil || req.Footer != "" || len(req.Buttons) > 0 || req.DisplayText != "" || req.URL != "" {
			return nil, cloudError("invalid_request", "text Direct Send cannot contain interactive fields")
		}
		body, err = cloudTextPayload(CloudTextRequest{ChatID: req.ChatID, Text: req.Text, ReplyToMessageID: req.ReplyToMessageID, RecipientIdentityKeyHash: req.RecipientIdentityKeyHash})
		if err == nil {
			delete(body["text"].(map[string]any), "preview_url")
		}
	case "buttons":
		if req.DisplayText != "" || req.URL != "" {
			return nil, cloudError("invalid_request", "buttons Direct Send cannot contain a standalone URL action")
		}
		var buttons []map[string]any
		buttons, err = cloudButtonsPayload(req.Buttons, "reply")
		if err == nil {
			body, err = cloudInteractivePayload(req.CloudInteractiveBase, "button", 1024, map[string]any{"buttons": buttons})
		}
	case "cta_url":
		if len(req.Buttons) > 0 {
			return nil, cloudError("invalid_request", "cta_url Direct Send cannot contain reply buttons")
		}
		var action map[string]any
		action, err = cloudCTAPayload(req.DisplayText, req.URL)
		if err == nil {
			body, err = cloudInteractivePayload(req.CloudInteractiveBase, "cta_url", 1024, action)
		}
	default:
		return nil, cloudError("invalid_request", "Direct Send type must be text, buttons or cta_url")
	}
	if err != nil {
		return nil, cloudError("invalid_request", err.Error())
	}
	body["category"] = req.Category
	if req.TTLSeconds != nil {
		body["ttl_seconds"] = *req.TTLSeconds
	}
	if req.TemplateName != "" {
		body["direct_send_config"] = map[string]any{"template_name": req.TemplateName}
	}
	return body, nil
}
func registerCloudMarketing(r *Registry, client *cloudClient) {
	r.Add("whatsapp::cloud::messages::send_marketing_template", func(ctx context.Context, input map[string]any) (any, error) {
		var req CloudMarketingTemplateRequest
		if err := decodeCloudRequest(input, &req); err != nil {
			return nil, cloudError("invalid_request", "Invalid marketing-template request")
		}
		body, err := cloudMarketingPayload(req)
		if err != nil {
			return nil, err
		}
		return cloudSendAndRetainAt(ctx, r, client, req.ChatID, body, "template", "[Marketing template: "+req.Name+"]", "marketing_messages")
	})
	r.Add("whatsapp::cloud::messages::send_direct", func(ctx context.Context, input map[string]any) (any, error) {
		var req CloudDirectSendRequest
		if err := decodeCloudRequest(input, &req); err != nil {
			return nil, cloudError("invalid_request", "Invalid Direct Send request")
		}
		body, err := cloudDirectPayload(req)
		if err != nil {
			return nil, err
		}
		return cloudSendAndRetain(ctx, r, client, req.ChatID, body, "direct_send", req.Text)
	})
}
