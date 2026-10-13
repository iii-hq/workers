package main

import (
	"context"
	"encoding/json"
	"fmt"
	"maps"
	"net"
	"net/http"
	"net/mail"
	"net/url"
	"regexp"
	"slices"
	"strconv"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"
)

// Management operations always use the configured phone/WABA. Template IDs are
// the only root Graph resources accepted, and are checked against that WABA.
type CloudProfileUpdateRequest struct {
	About                *string   `json:"about,omitempty"`
	Address              *string   `json:"address,omitempty"`
	Description          *string   `json:"description,omitempty"`
	Email                *string   `json:"email,omitempty"`
	Websites             *[]string `json:"websites,omitempty"`
	Vertical             *string   `json:"vertical,omitempty"`
	ProfilePictureHandle *string   `json:"profile_picture_handle,omitempty"`
}

type CloudAnalyticsRequest struct {
	Field            string   `json:"field"`
	Start            int64    `json:"start"`
	End              int64    `json:"end"`
	Granularity      string   `json:"granularity"`
	TemplateIDs      []string `json:"template_ids,omitempty"`
	TemplateGroupIDs []string `json:"template_group_ids,omitempty"`
	GroupIDs         []string `json:"group_ids,omitempty"`
	MetricTypes      []string `json:"metric_types,omitempty"`
}

type CloudConversationCommand struct {
	Name        string `json:"command_name"`
	Description string `json:"command_description"`
}

type cloudManagementPlan struct {
	method, path, result, templateID string
	query                            url.Values
	body                             any
	fields                           []string
}

var cloudManagementOperations = []string{
	"templates::create", "templates::get", "templates::update", "templates::delete", "templates::unpause",
	"profile::get", "profile::update", "phone_numbers::get", "phone_numbers::list", "phone_numbers::request_code",
	"phone_numbers::verify_code", "phone_numbers::register", "phone_numbers::deregister", "phone_numbers::set_pin", "phone_numbers::set_display_name",
	"subscriptions::list", "subscriptions::subscribe", "subscriptions::unsubscribe", "commerce::get", "commerce::update", "analytics::get",
	"qr_codes::create", "qr_codes::list", "qr_codes::get", "qr_codes::update", "qr_codes::delete",
	"conversational_components::get", "conversational_components::update", "blocking::list", "blocking::block", "blocking::unblock", "identity::get", "identity::update",
}

var cloudManagementQRCode = regexp.MustCompile(`^[A-Za-z0-9]{1,128}$`)
var cloudManagementPhoneFields = []string{"id", "display_phone_number", "verified_name", "quality_rating", "code_verification_status", "name_status", "new_display_name", "new_name_status", "throughput", "platform_type", "status", "is_official_business_account"}
var cloudManagementDefaultPhoneFields = []string{"id", "display_phone_number", "verified_name", "quality_rating", "code_verification_status"}
var cloudManagementTemplateFields = []string{"id", "name", "language", "status", "category", "components", "parameter_format", "quality_score", "rejected_reason", "message_send_ttl_seconds"}
var cloudManagementProfileFields = []string{"about", "address", "description", "email", "profile_picture_url", "websites", "vertical", "messaging_product"}
var cloudManagementQRFields = []string{"code", "prefilled_message", "deep_link_url", "qr_image_url"}

func cloudManagementInvalid() error {
	return cloudError("invalid_request", "Invalid Cloud management request; check the function's field constraints")
}

func cloudManagementDecode(input map[string]any, output any) error {
	if err := decodeCloudRequest(input, output); err != nil {
		return cloudManagementInvalid()
	}
	return nil
}

func cloudManagementText(value string, min, max int) bool {
	return utf8.ValidString(value) && utf8.RuneCountInString(value) >= min && utf8.RuneCountInString(value) <= max && !strings.ContainsRune(value, 0)
}

func cloudManagementURL(raw string, httpsOnly bool) bool {
	u, err := url.Parse(raw)
	if err != nil || len(raw) > 2048 || u.Hostname() == "" || u.User != nil || u.Fragment != "" || (u.Scheme != "https" && (httpsOnly || u.Scheme != "http")) {
		return false
	}
	host := strings.ToLower(strings.TrimSuffix(u.Hostname(), "."))
	if host == "localhost" || strings.HasSuffix(host, ".localhost") || strings.HasSuffix(host, ".local") {
		return false
	}
	if ip := net.ParseIP(host); ip != nil && (ip.IsPrivate() || ip.IsLoopback() || ip.IsUnspecified() || ip.IsLinkLocalUnicast() || ip.IsLinkLocalMulticast() || ip.IsMulticast()) {
		return false
	}
	for key := range u.Query() {
		if strings.Contains(strings.ToLower(key), "token") || strings.Contains(strings.ToLower(key), "secret") {
			return false
		}
	}
	return true
}

// cloudPageQuery decodes a {limit, after} page request; schemas bound limit.
func cloudPageQuery(input map[string]any, limit int) (url.Values, error) {
	req := CloudTemplatesRequest{Limit: limit}
	if err := decodeCloudRequest(input, &req); err != nil {
		return nil, err
	}
	if err := cloudCursor(req.After); err != nil {
		return nil, err
	}
	query := url.Values{"limit": {strconv.Itoa(req.Limit)}}
	if req.After != "" {
		query.Set("after", req.After)
	}
	return query, nil
}

func cloudManagementProfile(req CloudProfileUpdateRequest) error {
	for _, item := range []struct {
		value *string
		max   int
	}{{req.About, 139}, {req.Address, 256}, {req.Description, 512}, {req.Email, 128}, {req.ProfilePictureHandle, 8192}} {
		if item.value != nil && !cloudManagementText(*item.value, 0, item.max) {
			return cloudManagementInvalid()
		}
	}
	if req.Email != nil && *req.Email != "" {
		email, err := mail.ParseAddress(*req.Email)
		if err != nil || email.Address != *req.Email {
			return cloudManagementInvalid()
		}
	}
	if req.Websites != nil {
		for _, website := range *req.Websites {
			if len(website) > 256 || !cloudManagementURL(website, false) {
				return cloudManagementInvalid()
			}
		}
	}
	return nil
}

func cloudManagementAnalytics(config Config, req CloudAnalyticsRequest) (cloudManagementPlan, error) {
	p := cloudManagementPlan{method: http.MethodGet, path: config.Cloud.BusinessAccountID, result: "analytics", fields: []string{req.Field}}
	now := time.Now().Unix()
	if req.Start < 1 || req.End <= req.Start || req.End > now+300 {
		return p, cloudManagementInvalid()
	}
	allowedFields := []string{"analytics", "conversation_analytics", "pricing_analytics", "template_analytics", "template_group_analytics", "call_analytics", "group_analytics"}
	if !slices.Contains(allowedFields, req.Field) {
		return p, cloudManagementInvalid()
	}
	daily := slices.Contains([]string{"template_analytics", "template_group_analytics", "group_analytics"}, req.Field)
	if daily {
		if req.Granularity != "DAILY" || req.Start < now-90*86400 {
			return p, cloudManagementInvalid()
		}
	} else {
		grans := []string{"HALF_HOUR", "DAILY", "MONTHLY"}
		if req.Field == "analytics" {
			grans = []string{"HALF_HOUR", "DAY", "MONTH"}
		}
		if !slices.Contains(grans, req.Granularity) || req.Start < time.Now().AddDate(-1, 0, 0).Unix() {
			return p, cloudManagementInvalid()
		}
	}
	if len(req.TemplateIDs) > 0 && req.Field != "template_analytics" || len(req.TemplateGroupIDs) > 0 && req.Field != "template_group_analytics" || len(req.GroupIDs) > 0 && req.Field != "group_analytics" {
		return p, cloudManagementInvalid()
	}
	if req.Field == "template_analytics" && len(req.TemplateIDs) == 0 || req.Field == "template_group_analytics" && len(req.TemplateGroupIDs) == 0 || req.Field == "group_analytics" && (len(req.GroupIDs) != 1 || len(req.MetricTypes) == 0) {
		return p, cloudManagementInvalid()
	}
	metrics := map[string][]string{"template_analytics": {"COST", "CLICKED", "DELIVERED", "READ", "SENT"}, "template_group_analytics": {"cost", "clicked", "delivered", "read", "sent"}, "group_analytics": {"SENT", "DELIVERED", "READ", "PARTICIPANTS_JOINED", "PARTICIPANTS_LEFT"}, "conversation_analytics": {"COST", "CONVERSATION"}, "call_analytics": {"COUNT", "COST", "AVERAGE_DURATION"}}
	for _, metric := range req.MetricTypes {
		if !slices.Contains(metrics[req.Field], metric) {
			return p, cloudManagementInvalid()
		}
	}
	p.query = url.Values{}
	if daily {
		p.path += "/" + req.Field
		p.result = "analytics_edge"
		p.query.Set("start", strconv.FormatInt(req.Start, 10))
		p.query.Set("end", strconv.FormatInt(req.End, 10))
		p.query.Set("granularity", req.Granularity)
		for key, values := range map[string][]string{"template_ids": req.TemplateIDs, "template_group_ids": req.TemplateGroupIDs, "group_ids": req.GroupIDs, "metric_types": req.MetricTypes} {
			if len(values) > 0 {
				encoded, _ := json.Marshal(values)
				p.query.Set(key, string(encoded))
			}
		}
	} else {
		fields := fmt.Sprintf("%s.start(%d).end(%d).granularity(%s)", req.Field, req.Start, req.End, req.Granularity)
		if len(req.MetricTypes) > 0 {
			encoded, _ := json.Marshal(req.MetricTypes)
			fields += ".metric_types(" + string(encoded) + ")"
		}
		p.query.Set("fields", fields)
	}
	return p, nil
}

func cloudManagementPlanFor(config Config, operation string, input map[string]any) (cloudManagementPlan, error) {
	p := cloudManagementPlan{method: http.MethodPost, result: "success"}
	phone, waba := config.Cloud.PhoneNumberID, config.Cloud.BusinessAccountID
	switch operation {
	case "templates::create", "templates::update":
		// The request schemas forbid unknown keys, so the input is the Graph body.
		id := stringArg(input, "template_id")
		body := maps.Clone(input)
		delete(body, "template_id")
		// Worker policy bounds complex template payloads in addition to per-field schemas.
		if raw, _ := json.Marshal(body); len(raw) > 128*1024 {
			return p, cloudManagementInvalid()
		}
		p.body = body
		if operation == "templates::create" {
			p.path = waba + "/message_templates"
			p.result = "template"
			p.fields = cloudManagementTemplateFields
		} else if !cloudDocumentMediaID(id) {
			return p, cloudManagementInvalid()
		} else {
			p.path = id
			p.templateID = id
		}
	case "templates::get", "templates::unpause":
		var req struct {
			TemplateID string `json:"template_id"`
		}
		if err := cloudManagementDecode(input, &req); err != nil {
			return p, err
		}
		if !cloudDocumentMediaID(req.TemplateID) {
			return p, cloudManagementInvalid()
		}
		p.path = req.TemplateID
		p.templateID = req.TemplateID
		if operation == "templates::get" {
			p.method = http.MethodGet
			p.result = "template"
			p.fields = cloudManagementTemplateFields
			p.query = url.Values{"fields": {strings.Join(cloudManagementTemplateFields, ",")}}
		} else {
			p.path += "/unpause"
		}
	case "templates::delete":
		var req struct {
			Name       string `json:"name"`
			TemplateID string `json:"template_id"`
		}
		if err := cloudManagementDecode(input, &req); err != nil {
			return p, err
		}
		p.method = http.MethodDelete
		p.path = waba + "/message_templates"
		p.query = url.Values{"name": {req.Name}}
		if req.TemplateID != "" {
			p.query.Set("hsm_id", req.TemplateID)
		}
	case "profile::get":
		p.method = http.MethodGet
		p.path = phone + "/whatsapp_business_profile"
		p.query = url.Values{"fields": {strings.Join(cloudManagementProfileFields, ",")}}
		p.result = "profile"
	case "profile::update":
		var req CloudProfileUpdateRequest
		if err := cloudManagementDecode(input, &req); err != nil {
			return p, err
		}
		if err := cloudManagementProfile(req); err != nil {
			return p, err
		}
		p.path = phone + "/whatsapp_business_profile"
		body := maps.Clone(input)
		body["messaging_product"] = "whatsapp"
		p.body = body
	case "phone_numbers::get":
		var req struct {
			Fields []string `json:"fields"`
		}
		if err := cloudManagementDecode(input, &req); err != nil {
			return p, err
		}
		if len(req.Fields) == 0 {
			req.Fields = cloudManagementDefaultPhoneFields
		}
		p.method = http.MethodGet
		p.path = phone
		p.query = url.Values{"fields": {strings.Join(req.Fields, ",")}}
		p.result = "phone"
		p.fields = cloudManagementPhoneFields
	case "phone_numbers::list", "subscriptions::list", "qr_codes::list", "blocking::list":
		limit := 50
		if operation == "qr_codes::list" {
			limit = 25
		}
		query, err := cloudPageQuery(input, limit)
		if err != nil {
			return p, cloudManagementInvalid()
		}
		p.method = http.MethodGet
		p.query = query
		p.result = "page"
		switch operation {
		case "phone_numbers::list":
			p.path = waba + "/phone_numbers"
			p.query.Set("fields", strings.Join(cloudManagementDefaultPhoneFields, ","))
			p.fields = cloudManagementPhoneFields
		case "subscriptions::list":
			p.path = waba + "/subscribed_apps"
			p.fields = []string{"whatsapp_business_api_data", "override_callback_uri"}
		case "qr_codes::list":
			p.path = phone + "/message_qrdls"
			p.fields = cloudManagementQRFields
		case "blocking::list":
			p.path = phone + "/block_users"
			p.fields = []string{"wa_id", "input", "user_id"}
		}
	case "phone_numbers::request_code":
		var req struct {
			Method   string `json:"code_method"`
			Language string `json:"language"`
		}
		if err := cloudManagementDecode(input, &req); err != nil {
			return p, err
		}
		p.path = phone + "/request_code"
		p.body = map[string]any{"code_method": req.Method, "language": req.Language}
	case "phone_numbers::verify_code":
		var req struct {
			Code string `json:"code"`
		}
		if err := cloudManagementDecode(input, &req); err != nil {
			return p, err
		}
		p.path = phone + "/verify_code"
		p.body = map[string]any{"code": req.Code}
	case "phone_numbers::register", "phone_numbers::set_pin":
		var req struct {
			PIN    string `json:"pin"`
			Region string `json:"data_localization_region,omitempty"`
		}
		if err := cloudManagementDecode(input, &req); err != nil {
			return p, err
		}
		p.path = phone
		p.body = map[string]any{"pin": req.PIN}
		if operation == "phone_numbers::register" {
			p.path += "/register"
			body := p.body.(map[string]any)
			body["messaging_product"] = "whatsapp"
			if req.Region != "" {
				body["data_localization_region"] = req.Region
			}
		}
	case "phone_numbers::deregister", "subscriptions::unsubscribe":
		p.path = phone + "/deregister"
		if operation == "subscriptions::unsubscribe" {
			p.path = waba + "/subscribed_apps"
			p.method = http.MethodDelete
		}
	case "phone_numbers::set_display_name":
		var req struct {
			Name string `json:"new_display_name"`
		}
		if err := cloudManagementDecode(input, &req); err != nil {
			return p, err
		}
		if !cloudManagementText(req.Name, 1, 512) || strings.TrimSpace(req.Name) == "" {
			return p, cloudManagementInvalid()
		}
		p.path = phone
		p.body = map[string]any{"new_display_name": req.Name}
	case "subscriptions::subscribe":
		var req struct {
			Callback string `json:"override_callback_uri"`
		}
		if err := cloudManagementDecode(input, &req); err != nil {
			return p, err
		}
		p.path = waba + "/subscribed_apps"
		if req.Callback != "" {
			if !cloudManagementURL(req.Callback, true) {
				return p, cloudManagementInvalid()
			}
			if strings.TrimSpace(config.Cloud.VerifyToken) == "" {
				return p, cloudError("cloud_not_configured", "Webhook verify token is required to set a callback override")
			}
			p.body = map[string]any{"override_callback_uri": req.Callback, "verify_token": config.Cloud.VerifyToken}
		}
	case "commerce::get":
		p.method = http.MethodGet
		p.path = phone + "/whatsapp_commerce_settings"
		p.result = "commerce"
	case "commerce::update":
		var req struct {
			Cart    *bool `json:"is_cart_enabled,omitempty"`
			Catalog *bool `json:"is_catalog_visible,omitempty"`
		}
		if err := cloudManagementDecode(input, &req); err != nil {
			return p, err
		}
		p.path = phone + "/whatsapp_commerce_settings"
		p.body = req
	case "analytics::get":
		var req CloudAnalyticsRequest
		if err := cloudManagementDecode(input, &req); err != nil {
			return p, err
		}
		return cloudManagementAnalytics(config, req)
	case "qr_codes::create", "qr_codes::update":
		var req struct {
			Code    string `json:"code,omitempty"`
			Message string `json:"prefilled_message"`
			Format  string `json:"generate_qr_image,omitempty"`
		}
		if err := cloudManagementDecode(input, &req); err != nil {
			return p, err
		}
		if !cloudManagementText(req.Message, 0, 140) {
			return p, cloudManagementInvalid()
		}
		p.path = phone + "/message_qrdls"
		p.result = "qr"
		p.body = req
	case "qr_codes::get", "qr_codes::delete":
		var req struct {
			Code string `json:"code"`
		}
		if err := cloudManagementDecode(input, &req); err != nil {
			return p, err
		}
		if !cloudManagementQRCode.MatchString(req.Code) {
			return p, cloudManagementInvalid()
		}
		p.path = phone + "/message_qrdls"
		if operation == "qr_codes::get" {
			p.method = http.MethodGet
			p.result = "page"
			p.fields = cloudManagementQRFields
			p.query = url.Values{"code": {req.Code}}
		} else {
			p.method = http.MethodDelete
			p.path += "/" + req.Code
		}
	case "conversational_components::get":
		p.method = http.MethodGet
		p.path = phone
		p.query = url.Values{"fields": {"conversational_automation"}}
		p.result = "components"
	case "conversational_components::update":
		var req struct {
			Prompts  *[]string                   `json:"prompts,omitempty"`
			Commands *[]CloudConversationCommand `json:"commands,omitempty"`
		}
		if err := cloudManagementDecode(input, &req); err != nil {
			return p, err
		}
		noEmoji := func(value string) bool {
			return strings.IndexFunc(value, func(r rune) bool { return unicode.Is(unicode.So, r) || r == 0x200d || r == 0xfe0f || r == 0x20e3 }) < 0
		}
		if req.Prompts != nil {
			for _, prompt := range *req.Prompts {
				if !cloudManagementText(prompt, 1, 80) || !noEmoji(prompt) {
					return p, cloudManagementInvalid()
				}
			}
		}
		if req.Commands != nil {
			for _, command := range *req.Commands {
				if !cloudManagementText(command.Description, 1, 256) || !noEmoji(command.Description) {
					return p, cloudManagementInvalid()
				}
			}
		}
		p.path = phone + "/conversational_automation"
		p.body = req
	case "blocking::block", "blocking::unblock":
		var req struct {
			Users []string `json:"users"`
		}
		if err := cloudManagementDecode(input, &req); err != nil {
			return p, err
		}
		users := make([]map[string]any, 0, len(req.Users))
		for _, user := range req.Users {
			users = append(users, map[string]any{"user": user})
		}
		p.path = phone + "/block_users"
		p.body = map[string]any{"messaging_product": "whatsapp", "block_users": users}
		p.result = "blocking"
		if operation == "blocking::unblock" {
			p.method = http.MethodDelete
		}
	case "identity::get":
		p.method = http.MethodGet
		p.path = phone + "/settings"
		p.result = "identity"
	case "identity::update":
		var req struct {
			Enabled bool `json:"enable_identity_key_check"`
		}
		if err := cloudManagementDecode(input, &req); err != nil {
			return p, err
		}
		p.path = phone + "/settings"
		p.body = map[string]any{"user_identity_change": map[string]any{"enable_identity_key_check": req.Enabled}}
	default:
		return p, cloudManagementInvalid()
	}
	return p, nil
}

func cloudManagementPick(value map[string]any, fields []string) map[string]any {
	result := map[string]any{}
	for _, key := range fields {
		if item, ok := value[key]; ok {
			result[key] = item
		}
	}
	return result
}

func (c *cloudClient) manageCloud(ctx context.Context, config Config, operation string, input map[string]any) (any, error) {
	if err := cloudRequestConfig(config); err != nil {
		return nil, err
	}
	p, err := cloudManagementPlanFor(config, operation, input)
	if err != nil {
		return nil, err
	}
	if p.templateID != "" {
		templates, err := c.collection(ctx, config, config.Cloud.BusinessAccountID+"/message_templates", "id")
		if err != nil {
			return nil, err
		}
		found := false
		for _, item := range templates {
			if template, ok := item.(map[string]any); ok && template["id"] == p.templateID {
				found = true
				break
			}
		}
		if !found {
			return nil, cloudError("cloud_resource_not_found", "Template does not belong to the configured WhatsApp Business Account")
		}
	}
	value, err := c.request(ctx, config, p.method, p.path, p.query, p.body)
	if err != nil {
		return nil, err
	}
	var result any
	switch p.result {
	case "success":
		success, ok := value["success"].(bool)
		if !ok {
			return nil, cloudResponseError(p.method, "Cloud API returned no management acknowledgement")
		}
		result = map[string]any{"success": success}
	case "template", "phone":
		id, _ := value["id"].(string)
		if !cloudDocumentMediaID(id) {
			return nil, cloudResponseError(p.method, "Cloud API returned no valid "+p.result+" ID")
		}
		result = cloudManagementPick(value, p.fields)
	case "qr":
		code, _ := value["code"].(string)
		if !cloudManagementQRCode.MatchString(code) {
			return nil, cloudResponseError(p.method, "Cloud API returned no valid QR code")
		}
		result = cloudManagementPick(value, cloudManagementQRFields)
	case "page":
		items, err := cloudArray(value["data"])
		if err != nil {
			return nil, cloudError("cloud_invalid_response", "Cloud API returned an invalid management collection")
		}
		if len(items) > 1000 {
			return nil, cloudError("cloud_invalid_response", "Cloud API management collection exceeds the item limit")
		}
		clean := make([]any, 0, len(items))
		for _, item := range items {
			record, ok := item.(map[string]any)
			if !ok {
				return nil, cloudError("cloud_invalid_response", "Cloud API returned an invalid management record")
			}
			clean = append(clean, cloudManagementPick(record, p.fields))
		}
		cursor, err := cloudNextCursor(value)
		if err != nil {
			return nil, cloudError("cloud_invalid_response", "Cloud API returned an invalid management cursor")
		}
		// The documented blocklist response has cursors but no paging.next URL.
		if operation == "blocking::list" && cursor == nil && len(items) > 0 {
			paging, _ := value["paging"].(map[string]any)
			cursors, _ := paging["cursors"].(map[string]any)
			if after, ok := cursors["after"].(string); ok && after != "" {
				if cloudCursor(after) != nil {
					return nil, cloudError("cloud_invalid_response", "Cloud API returned an invalid management cursor")
				}
				cursor = &after
			}
		}
		var next any
		if cursor != nil {
			if *cursor == p.query.Get("after") {
				return nil, cloudError("cloud_invalid_response", "Cloud API returned a repeated management cursor")
			}
			next = *cursor
		}
		result = map[string]any{"items": clean, "next_cursor": next}
	case "profile", "commerce":
		items, err := cloudArray(value["data"])
		if err != nil || len(items) != 1 {
			return nil, cloudError("cloud_invalid_response", "Cloud API returned no unique business settings record")
		}
		record, ok := items[0].(map[string]any)
		if !ok {
			return nil, cloudError("cloud_invalid_response", "Cloud API returned invalid business settings")
		}
		if p.result == "profile" {
			result = map[string]any{"profile": cloudManagementPick(record, cloudManagementProfileFields)}
		} else {
			result = map[string]any{"settings": cloudManagementPick(record, []string{"id", "is_cart_enabled", "is_catalog_visible"})}
		}
	case "analytics":
		if _, ok := value[p.fields[0]].(map[string]any); !ok {
			return nil, cloudError("cloud_invalid_response", "Cloud API returned no analytics data")
		}
		result = map[string]any{"field": p.fields[0], "data": value[p.fields[0]]}
	case "analytics_edge":
		if _, ok := value["data"].([]any); !ok {
			return nil, cloudError("cloud_invalid_response", "Cloud API returned no analytics data")
		}
		result = map[string]any{"field": p.fields[0], "data": value["data"]}
	case "components", "identity":
		key, fields := "conversational_automation", []string{"prompts", "commands"}
		if p.result == "identity" {
			key, fields = "user_identity_change", []string{"enable_identity_key_check", "enabled"}
		}
		settings, ok := value[key].(map[string]any)
		if !ok && value[key] != nil {
			return nil, cloudError("cloud_invalid_response", "Cloud API returned invalid "+p.result+" settings")
		}
		result = map[string]any{"settings": nil}
		if settings != nil {
			result = map[string]any{"settings": cloudManagementPick(settings, fields)}
		}
	case "blocking":
		if _, ok := value["block_users"].(map[string]any); !ok {
			return nil, cloudResponseError(p.method, "Cloud API returned no blocklist acknowledgement")
		}
		result = map[string]any{"block_users": value["block_users"]}
	default:
		return nil, cloudError("cloud_invalid_response", "Cloud API management response cannot be decoded")
	}
	return cloudSanitizeResult(config, result), nil
}

func registerCloudManagement(r *Registry, client *cloudClient) {
	for _, operation := range cloudManagementOperations {
		r.Add("whatsapp::cloud::"+operation, func(ctx context.Context, input map[string]any) (any, error) {
			config, configErr := r.Manager.ConfigContext(ctx)
			if configErr != nil {
				return nil, configErr
			}
			return client.manageCloud(ctx, config, operation, input)
		})
	}
}
