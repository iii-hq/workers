package main

import (
	"bytes"
	"context"
	"encoding/base64"
	"image"
	"io"
	"net/http"
	"net/url"
	"strconv"
	"strings"
)

type CloudGroupIDRequest struct {
	GroupID string `json:"group_id"`
}
type CloudGroupCreateRequest struct {
	Subject          string `json:"subject"`
	Description      string `json:"description"`
	JoinApprovalMode string `json:"join_approval_mode"`
}
type CloudGroupUpdateRequest struct {
	GroupID     string  `json:"group_id"`
	Subject     *string `json:"subject"`
	Description *string `json:"description"`
}
type CloudGroupGetRequest struct {
	GroupID string   `json:"group_id"`
	Fields  []string `json:"fields"`
}
type CloudGroupJoinRequest struct {
	GroupID        string   `json:"group_id"`
	JoinRequestIDs []string `json:"join_request_ids"`
}
type CloudGroupJoinListRequest struct {
	GroupID string `json:"group_id"`
	Limit   int    `json:"limit"`
	After   string `json:"after"`
}
type CloudGroupParticipantsRequest struct {
	GroupID        string   `json:"group_id"`
	ParticipantIDs []string `json:"participant_ids"`
}
type CloudGroupPhotoRequest struct {
	GroupID    string `json:"group_id"`
	DataBase64 string `json:"data_base64"`
	FileName   string `json:"file_name"`
	Mimetype   string `json:"mimetype"`
}
type CloudGroupTextRequest struct {
	GroupID          string `json:"group_id"`
	Text             string `json:"text"`
	PreviewURL       bool   `json:"preview_url"`
	ReplyToMessageID string `json:"reply_to_message_id"`
}
type CloudGroupMediaRequest struct {
	GroupID          string  `json:"group_id"`
	MediaType        string  `json:"media_type"`
	MediaID          string  `json:"media_id"`
	FileName         string  `json:"file_name"`
	Caption          *string `json:"caption"`
	ReplyToMessageID string  `json:"reply_to_message_id"`
}
type CloudGroupTemplateRequest struct {
	GroupID          string           `json:"group_id"`
	Name             string           `json:"name"`
	Language         string           `json:"language"`
	Components       []map[string]any `json:"components"`
	ReplyToMessageID string           `json:"reply_to_message_id"`
}
type CloudGroupPinRequest struct {
	GroupID        string `json:"group_id"`
	MessageID      string `json:"message_id"`
	ExpirationDays int    `json:"expiration_days"`
}

var cloudGroupFields = []string{"join_approval_mode", "subject", "description", "suspended", "creation_timestamp", "participants", "total_participant_count"}
var cloudGroupOperations = []string{"list", "get", "create", "update", "delete", "photo::set", "invite_link::get", "invite_link::reset", "join_requests::list", "join_requests::approve", "join_requests::reject", "participants::remove"}

func cloudGroupInvalid() error {
	return cloudError("invalid_request", "Invalid Cloud group request; check the function's field constraints")
}
func cloudGroupID(id string) error {
	if !cloudGroupIDPattern.MatchString(id) {
		return cloudGroupInvalid()
	}
	return nil
}

// Root Graph group IDs cannot select another phone's resources through a broad
// account token. Discover ownership from the configured phone's own group edge.
func (c *cloudClient) requireCloudGroup(ctx context.Context, config Config, id string) error {
	if err := cloudGroupID(id); err != nil {
		return err
	}
	query := url.Values{"limit": {"1024"}}
	seen := map[string]bool{}
	for range 16 {
		response, err := c.request(ctx, config, http.MethodGet, config.Cloud.PhoneNumberID+"/groups", query, nil)
		if err != nil {
			return err
		}
		var groups []any
		if data, ok := response["data"].(map[string]any); ok {
			groups, _ = data["groups"].([]any)
		} else {
			groups, _ = response["data"].([]any)
		}
		for _, item := range groups {
			if group, ok := item.(map[string]any); ok && group["id"] == id {
				return nil
			}
		}
		paging, _ := response["paging"].(map[string]any)
		cursors, _ := paging["cursors"].(map[string]any)
		after, _ := cursors["after"].(string)
		if after == "" || len(groups) == 0 {
			break
		}
		if cloudCursor(after) != nil || seen[after] {
			return cloudError("cloud_invalid_response", "Invalid Cloud group pagination")
		}
		seen[after] = true
		query.Set("after", after)
	}
	return cloudError("cloud_group_not_found", "The group is not active under the configured Cloud API phone number")
}

func (c *cloudClient) manageGroup(ctx context.Context, config Config, operation string, input map[string]any) (map[string]any, error) {
	if err := cloudRequestConfig(config); err != nil {
		return nil, err
	}
	method, endpoint := http.MethodGet, config.Cloud.PhoneNumberID+"/groups"
	var groupID string
	var body any
	var query url.Values
	switch operation {
	case "list":
		var err error
		if query, err = cloudPageQuery(input, 25); err != nil {
			return nil, cloudGroupInvalid()
		}
	case "create":
		var req CloudGroupCreateRequest
		if decodeCloudRequest(input, &req) != nil || !cloudManagementText(strings.TrimSpace(req.Subject), 1, 128) || !cloudManagementText(req.Description, 0, 2048) || req.JoinApprovalMode != "" && req.JoinApprovalMode != "auto_approve" && req.JoinApprovalMode != "approval_required" {
			return nil, cloudGroupInvalid()
		}
		method = http.MethodPost
		data := map[string]any{"messaging_product": "whatsapp", "subject": strings.TrimSpace(req.Subject)}
		if req.Description != "" {
			data["description"] = req.Description
		}
		if req.JoinApprovalMode != "" {
			data["join_approval_mode"] = req.JoinApprovalMode
		}
		body = data
	case "update":
		var req CloudGroupUpdateRequest
		if decodeCloudRequest(input, &req) != nil || req.Subject == nil && req.Description == nil {
			return nil, cloudGroupInvalid()
		}
		data := map[string]any{"messaging_product": "whatsapp"}
		if req.Subject != nil {
			if !cloudManagementText(strings.TrimSpace(*req.Subject), 1, 128) {
				return nil, cloudGroupInvalid()
			}
			data["subject"] = strings.TrimSpace(*req.Subject)
		}
		if req.Description != nil {
			if !cloudManagementText(*req.Description, 0, 2048) {
				return nil, cloudGroupInvalid()
			}
			data["description"] = *req.Description
		}
		groupID = req.GroupID
		endpoint = url.PathEscape(groupID)
		method = http.MethodPost
		body = data
	case "get":
		var req CloudGroupGetRequest
		if decodeCloudRequest(input, &req) != nil {
			return nil, cloudGroupInvalid()
		}
		if len(req.Fields) == 0 {
			req.Fields = cloudGroupFields
		}
		groupID = req.GroupID
		endpoint = url.PathEscape(groupID)
		query = url.Values{"fields": {strings.Join(req.Fields, ",")}}
	case "join_requests::approve", "join_requests::reject":
		var req CloudGroupJoinRequest
		if decodeCloudRequest(input, &req) != nil {
			return nil, cloudGroupInvalid()
		}
		groupID = req.GroupID
		endpoint = url.PathEscape(groupID) + "/join_requests"
		method = http.MethodPost
		if operation == "join_requests::reject" {
			method = http.MethodDelete
		}
		body = map[string]any{"messaging_product": "whatsapp", "join_requests": req.JoinRequestIDs}
	case "participants::remove":
		var req CloudGroupParticipantsRequest
		if decodeCloudRequest(input, &req) != nil {
			return nil, cloudGroupInvalid()
		}
		participants := make([]map[string]any, 0, len(req.ParticipantIDs))
		for _, raw := range req.ParticipantIDs {
			id, err := normalizeCloudChatID(raw)
			if err != nil {
				return nil, cloudGroupInvalid()
			}
			participants = append(participants, map[string]any{"user": id})
		}
		groupID = req.GroupID
		endpoint = url.PathEscape(groupID) + "/participants"
		method = http.MethodDelete
		body = map[string]any{"messaging_product": "whatsapp", "participants": participants}
	case "photo::set":
		var req CloudGroupPhotoRequest
		if decodeCloudRequest(input, &req) != nil {
			return nil, cloudGroupInvalid()
		}
		return c.setCloudGroupPhoto(ctx, config, req)
	case "join_requests::list":
		req := CloudGroupJoinListRequest{Limit: 50}
		if decodeCloudRequest(input, &req) != nil || cloudCursor(req.After) != nil {
			return nil, cloudGroupInvalid()
		}
		groupID = req.GroupID
		endpoint = url.PathEscape(groupID) + "/join_requests"
		query = url.Values{"limit": {strconv.Itoa(req.Limit)}}
		if req.After != "" {
			query.Set("after", req.After)
		}
	case "delete", "invite_link::get", "invite_link::reset":
		var req CloudGroupIDRequest
		if decodeCloudRequest(input, &req) != nil {
			return nil, cloudGroupInvalid()
		}
		groupID = req.GroupID
		endpoint = url.PathEscape(groupID)
		switch operation {
		case "delete":
			method = http.MethodDelete
		case "invite_link::get":
			endpoint += "/invite_link"
		case "invite_link::reset":
			endpoint += "/invite_link"
			method = http.MethodPost
			body = map[string]any{"messaging_product": "whatsapp"}
		}
	default:
		return nil, cloudGroupInvalid()
	}
	if operation != "list" && operation != "create" {
		if err := c.requireCloudGroup(ctx, config, groupID); err != nil {
			return nil, err
		}
	}
	result, err := c.request(ctx, config, method, endpoint, query, body)
	if err != nil {
		return nil, err
	}
	if err := cloudGroupAcknowledgement(operation, method, result); err != nil {
		return nil, err
	}
	return cloudSanitizeResult(config, result).(map[string]any), nil
}

func cloudGroupAcknowledgement(operation, method string, result map[string]any) error {
	invalid := func() error {
		return cloudResponseError(method, "Cloud API returned no valid group acknowledgement; inspect group state and lifecycle webhooks before retrying")
	}
	// Most mutations complete asynchronously through group webhooks; do not
	// invent a universal success or group-ID field for those endpoints.
	if method != http.MethodGet && len(result) == 0 {
		return invalid()
	}
	switch operation {
	case "invite_link::get", "invite_link::reset":
		link, ok := result["invite_link"].(string)
		parsed, err := url.Parse(link)
		if !ok || err != nil || len(link) > 2048 || parsed.Scheme != "https" || parsed.Host != "chat.whatsapp.com" || parsed.User != nil || parsed.RawQuery != "" || parsed.Fragment != "" || len(parsed.Path) < 2 || strings.Contains(parsed.Path[1:], "/") {
			return invalid()
		}
	case "join_requests::approve", "join_requests::reject":
		key := "approved_join_requests"
		if operation == "join_requests::reject" {
			key = "rejected_join_requests"
		}
		accepted, acceptedOK := result[key].([]any)
		failed, failedOK := result["failed_join_requests"].([]any)
		if !acceptedOK && !failedOK {
			return invalid()
		}
		if len(accepted)+len(failed) > 100 {
			return invalid()
		}
		for _, item := range accepted {
			id, ok := item.(string)
			if !ok || cloudGroupID(id) != nil {
				return invalid()
			}
		}
		for _, item := range failed {
			if _, ok := item.(map[string]any); !ok {
				return invalid()
			}
		}
	}
	return nil
}

func (c *cloudClient) setCloudGroupPhoto(ctx context.Context, config Config, req CloudGroupPhotoRequest) (map[string]any, error) {
	_, spec, _ := cloudMediaMIME(req.Mimetype)
	if err := cloudMediaFilename(req.FileName, req.Mimetype, spec); err != nil {
		return nil, err
	}
	limit := min(config.MaxMediaBytes, 5_000_000)
	if limit < 1 || len(req.DataBase64) > base64.StdEncoding.EncodedLen(limit) {
		return nil, cloudError("cloud_media_too_large", "Group photo exceeds max_media_bytes or 5 MB")
	}
	data, err := decodeMedia(req.DataBase64, limit)
	if err != nil {
		return nil, cloudGroupInvalid()
	}
	if _, err := cloudMediaContent(data, req.FileName, req.Mimetype); err != nil {
		return nil, err
	}
	imageConfig, _, err := image.DecodeConfig(bytes.NewReader(data))
	if err != nil || imageConfig.Width != imageConfig.Height || imageConfig.Width < 192 {
		return nil, cloudError("invalid_request", "Group photos must be square JPEG images of at least 192 by 192 pixels")
	}
	if err := c.requireCloudGroup(ctx, config, req.GroupID); err != nil {
		return nil, err
	}
	body, contentType := cloudMultipartFile(req.FileName, "image/jpeg", data, "messaging_product", "whatsapp")
	envelope, _ := io.ReadAll(body) // buffered so the request keeps its Content-Length
	result, err := c.requestBody(ctx, config, http.MethodPost, url.PathEscape(req.GroupID), nil, bytes.NewReader(envelope), contentType)
	if err != nil {
		return nil, err
	}
	if err := cloudGroupAcknowledgement("photo::set", http.MethodPost, result); err != nil {
		return nil, err
	}
	return cloudSanitizeResult(config, result).(map[string]any), nil
}

func cloudGroupMessagePayload(operation string, input map[string]any) (string, map[string]any, string, string, error) {
	var groupID, kind, preview string
	var body map[string]any
	var err error
	// Reuse the existing type-specific validators, then replace the recipient
	// with the explicit group transport. The placeholder never leaves this function.
	const placeholder = "+15551234567"
	switch operation {
	case "send_text":
		var req CloudGroupTextRequest
		if decodeCloudRequest(input, &req) != nil {
			return "", nil, "", "", cloudGroupInvalid()
		}
		groupID, kind, preview = req.GroupID, "text", req.Text
		body, err = cloudTextPayload(CloudTextRequest{ChatID: placeholder, Text: req.Text, PreviewURL: req.PreviewURL, ReplyToMessageID: req.ReplyToMessageID})
	case "send_media":
		var req CloudGroupMediaRequest
		if decodeCloudRequest(input, &req) != nil {
			return "", nil, "", "", cloudGroupInvalid()
		}
		groupID, kind, preview = req.GroupID, req.MediaType, req.MediaType
		if req.Caption != nil {
			preview = *req.Caption
		}
		switch req.MediaType {
		case "document":
			body, err = cloudDocumentPayload(CloudDocumentRequest{ChatID: placeholder, MediaID: req.MediaID, FileName: req.FileName, Caption: req.Caption, ReplyToMessageID: req.ReplyToMessageID})
		case "image", "audio", "video", "sticker":
			if req.FileName != "" {
				return "", nil, "", "", cloudGroupInvalid()
			}
			body, err = cloudMediaMessagePayload(req.MediaType, CloudMediaMessageRequest{ChatID: placeholder, MediaID: req.MediaID, Caption: req.Caption, ReplyToMessageID: req.ReplyToMessageID})
		default:
			return "", nil, "", "", cloudGroupInvalid()
		}
	case "send_template":
		var req CloudGroupTemplateRequest
		if decodeCloudRequest(input, &req) != nil {
			return "", nil, "", "", cloudGroupInvalid()
		}
		groupID, kind, preview = req.GroupID, "template", req.Name
		body, err = cloudTemplatePayload(CloudTemplateRequest{ChatID: placeholder, Name: req.Name, Language: req.Language, Components: req.Components, ReplyToMessageID: req.ReplyToMessageID})
	case "pin", "unpin":
		var req CloudGroupPinRequest
		if decodeCloudRequest(input, &req) != nil {
			return "", nil, "", "", cloudGroupInvalid()
		}
		if err := cloudApplyReply(map[string]any{}, req.MessageID); err != nil {
			return "", nil, "", "", err
		}
		groupID, kind, preview = req.GroupID, "pin", operation
		pin := map[string]any{"type": operation, "message_id": req.MessageID}
		if operation == "pin" {
			pin["expiration_days"] = req.ExpirationDays
		}
		body = map[string]any{"messaging_product": "whatsapp", "type": "pin", "pin": pin}
	default:
		return "", nil, "", "", cloudGroupInvalid()
	}
	if err != nil {
		return "", nil, "", "", cloudGroupInvalid()
	}
	body["recipient_type"], body["to"] = "group", groupID
	delete(body, "recipient")
	return groupID, body, kind, preview, nil
}

func registerCloudGroups(r *Registry, client *cloudClient) {
	for _, operation := range cloudGroupOperations {
		r.Add("whatsapp::cloud::groups::"+operation, func(ctx context.Context, input map[string]any) (any, error) {
			config, configErr := r.Manager.ConfigContext(ctx)
			if configErr != nil {
				return nil, configErr
			}
			return client.manageGroup(ctx, config, operation, input)
		})
	}
	for _, operation := range []string{"send_text", "send_media", "send_template", "pin", "unpin"} {
		r.Add("whatsapp::cloud::groups::messages::"+operation, func(ctx context.Context, input map[string]any) (any, error) {
			group, body, kind, preview, err := cloudGroupMessagePayload(operation, input)
			if err != nil {
				return nil, err
			}
			config, configErr := r.Manager.ConfigContext(ctx)
			if configErr != nil {
				return nil, configErr
			}
			if err := client.requireCloudGroup(ctx, config, group); err != nil {
				return nil, err
			}
			return cloudSendAndRetain(ctx, r, client, "group:"+group, body, kind, preview)
		})
	}
}
