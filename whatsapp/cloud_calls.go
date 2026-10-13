package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/url"
	"regexp"
	"slices"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"
)

type CloudCallSession struct {
	SDPType string `json:"sdp_type"`
	SDP     string `json:"sdp"`
}
type CloudCallConnectRequest struct {
	ChatID       string           `json:"chat_id"`
	Session      CloudCallSession `json:"session"`
	CallbackData string           `json:"biz_opaque_callback_data,omitempty"`
}
type CloudCallAcceptRequest struct {
	CallID       string            `json:"call_id"`
	Session      *CloudCallSession `json:"session,omitempty"`
	CallbackData string            `json:"biz_opaque_callback_data,omitempty"`
}
type CloudCallIDRequest struct {
	CallID string `json:"call_id"`
}
type CloudCallPermissionRequest struct {
	ChatID string `json:"chat_id"`
}
type CloudCallPermissionMessageRequest struct {
	ChatID                   string `json:"chat_id"`
	Text                     string `json:"text"`
	ReplyToMessageID         string `json:"reply_to_message_id"`
	RecipientIdentityKeyHash string `json:"recipient_identity_key_hash"`
}
type CloudCallResult struct {
	CallID string `json:"call_id"`
	Action string `json:"action"`
	Status string `json:"status"`
}
type CloudCallHour struct {
	DayOfWeek string `json:"day_of_week"`
	OpenTime  string `json:"open_time"`
	CloseTime string `json:"close_time"`
}
type CloudCallHoliday struct {
	Date      string `json:"date"`
	StartTime string `json:"start_time"`
	EndTime   string `json:"end_time"`
}
type CloudCallHours struct {
	Status               string             `json:"status"`
	TimezoneID           string             `json:"timezone_id,omitempty"`
	WeeklyOperatingHours []CloudCallHour    `json:"weekly_operating_hours,omitempty"`
	HolidaySchedule      []CloudCallHoliday `json:"holiday_schedule,omitempty"`
}
type CloudSIPServer struct {
	Hostname             string            `json:"hostname"`
	Port                 int               `json:"port,omitempty"`
	RequestURIUserParams map[string]string `json:"request_uri_user_params,omitempty"`
}
type CloudSIPSettings struct {
	Status  string            `json:"status,omitempty"`
	Servers *[]CloudSIPServer `json:"servers,omitempty"`
}
type CloudCallIcons struct {
	RestrictToUserCountries []string `json:"restrict_to_user_countries"`
}
type CloudCallAudio struct {
	AdditionalCodecs []string `json:"additional_codecs"`
}
type CloudVoicemailDefault struct {
	AnnouncementMediaID json.Number `json:"announcement_media_id"`
	TimeoutSeconds      *int        `json:"timeout_seconds,omitempty"`
}
type CloudVoicemailAudio struct {
	Default CloudVoicemailDefault `json:"default"`
}
type CloudVoicemail struct {
	Status   string               `json:"status"`
	Triggers []string             `json:"triggers,omitempty"`
	Audio    *CloudVoicemailAudio `json:"audio,omitempty"`
}
type CloudCallingSettings struct {
	Status                   string            `json:"status,omitempty"`
	CallIconVisibility       string            `json:"call_icon_visibility,omitempty"`
	CallbackPermissionStatus string            `json:"callback_permission_status,omitempty"`
	CallIcons                *CloudCallIcons   `json:"call_icons,omitempty"`
	CallHours                *CloudCallHours   `json:"call_hours,omitempty"`
	SIP                      *CloudSIPSettings `json:"sip,omitempty"`
	Audio                    *CloudCallAudio   `json:"audio,omitempty"`
	Voicemail                *CloudVoicemail   `json:"voicemail,omitempty"`
}
type CloudCallSettingsRequest struct {
	Calling CloudCallingSettings `json:"calling"`
}

var cloudCallIDPattern = regexp.MustCompile(`^wacid\.[A-Za-z0-9_+/=-]+$`)
var cloudSIPHostPattern = regexp.MustCompile(`^[A-Za-z0-9](?:[A-Za-z0-9.-]{0,251}[A-Za-z0-9])?$`)

func cloudCallInvalid() error {
	return cloudError("invalid_request", "Invalid Cloud Calling API request; check the function's field constraints")
}
func cloudCallValidID(id string) bool { return len(id) <= 1024 && cloudCallIDPattern.MatchString(id) }

func cloudCallPayload(action string, input map[string]any) (map[string]any, error) {
	body := map[string]any{"messaging_product": "whatsapp", "action": action}
	var session *CloudCallSession
	var callback string
	switch action {
	case "connect":
		var req CloudCallConnectRequest
		if decodeCloudRequest(input, &req) != nil {
			return nil, cloudCallInvalid()
		}
		recipient, err := cloudRecipient(req.ChatID)
		if err != nil {
			return nil, cloudCallInvalid()
		}
		delete(recipient, "recipient_type")
		body = recipient
		body["action"] = action
		session = &req.Session
		callback = req.CallbackData
	case "accept", "pre_accept":
		var req CloudCallAcceptRequest
		if decodeCloudRequest(input, &req) != nil {
			return nil, cloudCallInvalid()
		}
		body["call_id"] = req.CallID
		session = req.Session
		callback = req.CallbackData
	case "reject", "terminate":
		var req CloudCallIDRequest
		if decodeCloudRequest(input, &req) != nil {
			return nil, cloudCallInvalid()
		}
		body["call_id"] = req.CallID
	default:
		return nil, cloudCallInvalid()
	}
	if session != nil {
		if len(session.SDP) > 64*1024 || !utf8.ValidString(session.SDP) || !strings.HasPrefix(session.SDP, "v=0\n") && !strings.HasPrefix(session.SDP, "v=0\r\n") || strings.IndexFunc(session.SDP, func(r rune) bool { return unicode.IsControl(r) && r != '\r' && r != '\n' && r != '\t' }) >= 0 {
			return nil, cloudCallInvalid()
		}
		body["session"] = session
	}
	if !cloudManagementText(callback, 0, 512) {
		return nil, cloudCallInvalid()
	}
	if callback != "" {
		body["biz_opaque_callback_data"] = callback
	}
	return body, nil
}

func (c *cloudClient) signalCloudCall(ctx context.Context, config Config, action string, input map[string]any) (CloudCallResult, error) {
	var result CloudCallResult
	body, err := cloudCallPayload(action, input)
	if err != nil {
		return result, err
	}
	response, err := c.request(ctx, config, http.MethodPost, config.Cloud.PhoneNumberID+"/calls", nil, body)
	if err != nil {
		return result, err
	}
	id, _ := body["call_id"].(string)
	if action == "connect" {
		calls, _ := response["calls"].([]any)
		if len(calls) == 1 {
			call, _ := calls[0].(map[string]any)
			id, _ = call["id"].(string)
		}
	} else if response["success"] != true {
		return result, cloudError("cloud_outcome_unknown", "Cloud API did not confirm the call operation; no automatic retry was made")
	}
	if !cloudCallValidID(id) || cloudSanitizeResult(config, id) != id {
		return result, cloudError("cloud_outcome_unknown", "Cloud API returned no valid call ID; the call may have started. No automatic retry was made")
	}
	return CloudCallResult{id, action, "accepted"}, nil
}

func cloudCallSettingsBody(req CloudCallSettingsRequest) (any, error) {
	c := req.Calling
	if c.CallHours != nil {
		h := c.CallHours
		if h.Status == "ENABLED" && (h.TimezoneID == "" || len(h.WeeklyOperatingHours) == 0) {
			return nil, cloudCallInvalid()
		}
		if h.TimezoneID != "" {
			if _, err := time.LoadLocation(h.TimezoneID); err != nil {
				return nil, cloudCallInvalid()
			}
		}
		intervals := map[string][][2]string{}
		add := func(key, start, end string) bool {
			if start >= end || len(intervals[key]) >= 2 {
				return false
			}
			for _, v := range intervals[key] {
				if start < v[1] && end > v[0] {
					return false
				}
			}
			intervals[key] = append(intervals[key], [2]string{start, end})
			return true
		}
		for _, entry := range h.WeeklyOperatingHours {
			if !add(entry.DayOfWeek, entry.OpenTime, entry.CloseTime) {
				return nil, cloudCallInvalid()
			}
		}
		for _, entry := range h.HolidaySchedule {
			if _, err := time.Parse("2006-01-02", entry.Date); err != nil || !add(entry.Date, entry.StartTime, entry.EndTime) {
				return nil, cloudCallInvalid()
			}
		}
	}
	if c.SIP != nil && c.SIP.Servers != nil {
		for _, server := range *c.SIP.Servers {
			if !cloudSIPHostPattern.MatchString(server.Hostname) || !cloudManagementURL("https://"+server.Hostname, true) {
				return nil, cloudCallInvalid()
			}
			for key, value := range server.RequestURIUserParams {
				if !cloudManagementText(key, 1, 128) || !cloudManagementText(value, 0, 128) || strings.ContainsAny(key+value, "\r\n;@\x00") {
					return nil, cloudCallInvalid()
				}
			}
		}
	}
	if c.Voicemail != nil {
		v := c.Voicemail
		if v.Status == "ENABLED" && (len(v.Triggers) == 0 || v.Audio == nil) {
			return nil, cloudCallInvalid()
		}
		if v.Audio != nil && slices.Contains(v.Triggers, "TIMEOUT") && v.Audio.Default.TimeoutSeconds == nil {
			return nil, cloudCallInvalid()
		}
		if v.Status == "ENABLED" && c.CallHours != nil && c.CallHours.Status == "ENABLED" {
			return nil, cloudError("invalid_request", "Disable call hours before enabling voicemail")
		}
	}
	raw, err := json.Marshal(req)
	if err != nil || len(raw) > 64*1024 {
		return nil, cloudCallInvalid()
	}
	return req, nil
}

// Settings responses must not expose Meta-generated SIP credentials even if a
// remote response unexpectedly includes them without include_sip_credentials.
func cloudCallSafeSettings(value any) any {
	switch v := value.(type) {
	case map[string]any:
		for key, item := range v {
			lower := strings.ToLower(key)
			if strings.Contains(lower, "password") || strings.Contains(lower, "credential") {
				delete(v, key)
			} else {
				v[key] = cloudCallSafeSettings(item)
			}
		}
	case []any:
		for i, item := range v {
			v[i] = cloudCallSafeSettings(item)
		}
	}
	return value
}

func (c *cloudClient) manageCloudCall(ctx context.Context, config Config, operation string, input map[string]any) (map[string]any, error) {
	method, path := http.MethodGet, config.Cloud.PhoneNumberID+"/settings"
	var body any
	var query url.Values
	switch operation {
	case "settings::get":
	case "settings::update":
		var req CloudCallSettingsRequest
		if decodeCloudRequest(input, &req) != nil {
			return nil, cloudCallInvalid()
		}
		var err error
		body, err = cloudCallSettingsBody(req)
		if err != nil {
			return nil, err
		}
		method = http.MethodPost
	case "permissions::get":
		var req CloudCallPermissionRequest
		if decodeCloudRequest(input, &req) != nil {
			return nil, cloudCallInvalid()
		}
		id, err := normalizeCloudChatID(req.ChatID)
		if err != nil {
			return nil, cloudCallInvalid()
		}
		path = config.Cloud.PhoneNumberID + "/call_permissions"
		query = url.Values{}
		if strings.HasPrefix(id, "+") {
			query.Set("user_wa_id", strings.TrimPrefix(id, "+"))
		} else {
			query.Set("recipient", id)
		}
	default:
		return nil, cloudCallInvalid()
	}
	response, err := c.request(ctx, config, method, path, query, body)
	if err != nil {
		return nil, err
	}
	if method == http.MethodPost && response["success"] != true {
		return nil, cloudError("cloud_outcome_unknown", "Cloud API did not confirm the settings update; no automatic retry was made")
	}
	return cloudCallSafeSettings(cloudSanitizeResult(config, response)).(map[string]any), nil
}

func cloudCallPermissionPayload(req CloudCallPermissionMessageRequest) (map[string]any, error) {
	body, err := cloudRecipient(req.ChatID)
	if err != nil || cloudFieldText("text", req.Text, 1024, false) != nil {
		return nil, cloudCallInvalid()
	}
	interactive := map[string]any{"type": "call_permission_request", "action": map[string]any{"name": "call_permission_request"}}
	if req.Text != "" {
		interactive["body"] = map[string]any{"text": req.Text}
	}
	body["type"], body["interactive"] = "interactive", interactive
	if cloudApplyReply(body, req.ReplyToMessageID) != nil || cloudApplyIdentity(body, req.RecipientIdentityKeyHash) != nil {
		return nil, cloudCallInvalid()
	}
	return body, nil
}

func registerCloudCalls(r *Registry, client *cloudClient) {
	for _, operation := range []string{"settings::get", "settings::update", "permissions::get"} {
		r.Add("whatsapp::cloud::calls::"+operation, func(ctx context.Context, input map[string]any) (any, error) {
			config, err := r.Manager.ConfigContext(ctx)
			if err != nil {
				return nil, err
			}
			return client.manageCloudCall(ctx, config, operation, input)
		})
	}
	for _, action := range []string{"connect", "pre_accept", "accept", "reject", "terminate"} {
		r.Add("whatsapp::cloud::calls::"+action, func(ctx context.Context, input map[string]any) (any, error) {
			config, err := r.Manager.ConfigContext(ctx)
			if err != nil {
				return nil, err
			}
			return client.signalCloudCall(ctx, config, action, input)
		})
	}
	r.Add("whatsapp::cloud::calls::permissions::request", func(ctx context.Context, input map[string]any) (any, error) {
		var req CloudCallPermissionMessageRequest
		if decodeCloudRequest(input, &req) != nil {
			return nil, cloudCallInvalid()
		}
		body, err := cloudCallPermissionPayload(req)
		if err != nil {
			return nil, err
		}
		return cloudSendAndRetain(ctx, r, client, req.ChatID, body, "interactive", req.Text)
	})
}
