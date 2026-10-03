package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"regexp"
	"strconv"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

type CloudConfig struct {
	Enabled           bool   `json:"enabled" yaml:"enabled"`
	APIVersion        string `json:"api_version" yaml:"api_version"`
	BusinessAccountID string `json:"business_account_id" yaml:"business_account_id"`
	PhoneNumberID     string `json:"phone_number_id" yaml:"phone_number_id"`
	AccessToken       string `json:"access_token" yaml:"access_token"`
	AppSecret         string `json:"app_secret" yaml:"app_secret"`
	VerifyToken       string `json:"verify_token" yaml:"verify_token"`
}

func DefaultCloudConfig() CloudConfig { return CloudConfig{APIVersion: "v26.0"} }
func (c CloudConfig) Redacted() CloudConfig {
	c.AccessToken = ""
	c.AppSecret = ""
	c.VerifyToken = ""
	return c
}
func (c CloudConfig) String() string {
	return fmt.Sprintf("CloudConfig(enabled=%t, version=%s, access_token_set=%t, app_secret_set=%t, verify_token_set=%t)", c.Enabled, c.APIVersion, c.AccessToken != "", c.AppSecret != "", c.VerifyToken != "")
}
func (c CloudConfig) GoString() string { return c.String() }

var cloudVersionPattern = regexp.MustCompile(`^v[0-9]{1,3}\.[0-9]{1,2}$`)
var cloudMetaIDPattern = regexp.MustCompile(`^[0-9]{1,32}$`)
var cloudBSUIDPattern = regexp.MustCompile(`^[A-Z]{2}\.(ENT\.)?[A-Za-z0-9]{1,128}$`)

func (c CloudConfig) Validate() error {
	if !cloudVersionPattern.MatchString(c.APIVersion) {
		return errors.New("cloud.api_version must use a version such as v26.0")
	}
	for name, value := range map[string]string{"business_account_id": c.BusinessAccountID, "phone_number_id": c.PhoneNumberID} {
		if value != "" && !cloudMetaIDPattern.MatchString(value) {
			return fmt.Errorf("cloud.%s must be a numeric Meta identifier", name)
		}
		if c.Enabled && value == "" {
			return fmt.Errorf("cloud.%s is required when Cloud API is enabled", name)
		}
	}
	if c.Enabled && strings.TrimSpace(c.AccessToken) == "" {
		return errors.New("cloud.access_token is required when Cloud API is enabled")
	}
	for name, value := range map[string]string{"access_token": c.AccessToken, "app_secret": c.AppSecret, "verify_token": c.VerifyToken} {
		if len(value) > 16384 || strings.ContainsAny(value, "\r\n") {
			return fmt.Errorf("cloud.%s must be a single line of at most 16384 bytes", name)
		}
	}
	return nil
}
func normalizeCloudChatID(raw string) (string, error) {
	raw = strings.TrimSpace(raw)
	if strings.Contains(raw, ".") {
		if !cloudBSUIDPattern.MatchString(raw) {
			return "", errors.New("invalid Cloud API business-scoped user ID")
		}
		return raw, nil
	}
	var b strings.Builder
	for _, r := range raw {
		if r != ' ' && r != '(' && r != ')' && r != '-' {
			b.WriteRune(r)
		}
	}
	digits := strings.TrimPrefix(b.String(), "+")
	if len(digits) < 7 || len(digits) > 15 || digits[0] == '0' {
		return "", errors.New("Cloud API recipient must be an international phone number or business-scoped user ID; native JIDs are not supported")
	}
	for _, r := range digits {
		if r < '0' || r > '9' {
			return "", errors.New("invalid Cloud API recipient")
		}
	}
	return "+" + digits, nil
}
func cloudStorageChatID(raw string) (string, error) {
	id, err := normalizeCloudChatID(raw)
	if err != nil {
		return "", err
	}
	return "cloud:" + id, nil
}
func cloudWebhookPath() string {
	var b strings.Builder
	for _, c := range []byte(ConfigID()) {
		if c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9' || strings.ContainsRune("-._~", rune(c)) {
			b.WriteByte(c)
		} else {
			fmt.Fprintf(&b, "%%%02X", c)
		}
	}
	return "/webhooks/whatsapp-cloud/" + b.String()
}

type CloudStatus struct {
	Enabled           bool     `json:"enabled"`
	Configured        bool     `json:"configured"`
	MissingFields     []string `json:"missing_fields"`
	APIVersion        string   `json:"api_version"`
	BusinessAccountID string   `json:"business_account_id"`
	PhoneNumberID     string   `json:"phone_number_id"`
	AccessTokenSet    bool     `json:"access_token_set"`
	AppSecretSet      bool     `json:"app_secret_set"`
	VerifyTokenSet    bool     `json:"verify_token_set"`
	WebhookPath       string   `json:"webhook_path"`
	WebhookReady      bool     `json:"webhook_ready"`
}

func cloudStatus(config Config) CloudStatus {
	c := config.Cloud
	s := CloudStatus{Enabled: c.Enabled, MissingFields: []string{}, APIVersion: c.APIVersion, BusinessAccountID: c.BusinessAccountID, PhoneNumberID: c.PhoneNumberID, AccessTokenSet: strings.TrimSpace(c.AccessToken) != "", AppSecretSet: strings.TrimSpace(c.AppSecret) != "", VerifyTokenSet: strings.TrimSpace(c.VerifyToken) != "", WebhookPath: cloudWebhookPath()}
	for _, field := range []struct{ name, value string }{{"business_account_id", c.BusinessAccountID}, {"phone_number_id", c.PhoneNumberID}, {"access_token", c.AccessToken}, {"app_secret", c.AppSecret}, {"verify_token", c.VerifyToken}} {
		if strings.TrimSpace(field.value) == "" {
			s.MissingFields = append(s.MissingFields, field.name)
		}
	}
	s.Configured = s.Enabled && len(s.MissingFields) == 0
	s.WebhookReady = s.Enabled && s.AppSecretSet && s.VerifyTokenSet
	return s
}

type cloudClient struct {
	http *http.Client
	base string
}

func newCloudClient() *cloudClient {
	// Fresh connections prevent net/http's implicit retry of a reused connection.
	transport := &http.Transport{Proxy: http.ProxyFromEnvironment, DialContext: (&net.Dialer{Timeout: 15 * time.Second}).DialContext, TLSHandshakeTimeout: 15 * time.Second, DisableKeepAlives: true, MaxResponseHeaderBytes: 65536}
	return &cloudClient{http: &http.Client{Transport: transport, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}, base: "https://graph.facebook.com"}
}
func (c *cloudClient) request(ctx context.Context, config Config, method, path string, query url.Values, body any) (map[string]any, error) {
	var reader io.Reader
	contentType := ""
	if body != nil {
		encoded, err := json.Marshal(body)
		if err != nil {
			return nil, cloudError("invalid_request", "Invalid Cloud API JSON request")
		}
		reader = bytes.NewReader(encoded)
		contentType = "application/json"
	}
	return c.requestBody(ctx, config, method, path, query, reader, contentType)
}

func cloudError(code, message string) *iii.InvocationError {
	return &iii.InvocationError{Code: code, Message: message}
}

// POST failures after dispatch cannot establish whether Meta accepted a message
// or uploaded media. Never replay a request, including unreadable acknowledgements.
func cloudResponseError(method, message string) error {
	if method == http.MethodPost {
		return cloudError("cloud_outcome_unknown", message+"; Meta may have completed the operation. No automatic retry was made; check messages before retrying a send")
	}
	return cloudError("cloud_invalid_response", message)
}

func cloudRequestConfig(config Config) error {
	if !config.Cloud.Enabled {
		return cloudError("cloud_disabled", "Cloud API is disabled")
	}
	if strings.TrimSpace(config.Cloud.AccessToken) == "" {
		return cloudError("cloud_not_configured", "Cloud API access token is missing")
	}
	if err := config.Cloud.Validate(); err != nil {
		return cloudError("cloud_not_configured", err.Error())
	}
	return nil
}

// JSON messages and multipart media share authentication, deadlines, response
// bounds and the transport that disables redirects and connection replay.
func (c *cloudClient) requestBody(ctx context.Context, config Config, method, path string, query url.Values, reader io.Reader, contentType string) (map[string]any, error) {
	if err := cloudRequestConfig(config); err != nil {
		return nil, err
	}
	ctx, cancel := context.WithTimeout(ctx, time.Duration(config.OperationTimeoutSecs)*time.Second)
	defer cancel()
	if ctx.Err() != nil {
		return nil, cloudError("cloud_request_canceled", "Cloud API request canceled before dispatch")
	}
	u := c.base + "/" + config.Cloud.APIVersion + "/" + path
	if len(query) > 0 {
		u += "?" + query.Encode()
	}
	req, err := http.NewRequestWithContext(ctx, method, u, reader)
	if err != nil {
		return nil, cloudError("invalid_request", "Invalid Cloud API request")
	}
	req.Header.Set("Authorization", "Bearer "+config.Cloud.AccessToken)
	req.Header.Set("User-Agent", "iii-whatsapp-cloud/0.1")
	if contentType != "" {
		req.Header.Set("Content-Type", contentType)
	}
	req.GetBody = nil
	response, err := c.http.Do(req)
	if err != nil {
		return nil, cloudResponseError(method, "Cloud API request failed or timed out")
	}
	defer response.Body.Close()
	const limit = 2 * 1024 * 1024
	if response.ContentLength > limit {
		return nil, cloudResponseError(method, "Cloud API response exceeds the size limit")
	}
	raw, err := io.ReadAll(io.LimitReader(response.Body, limit+1))
	if err != nil {
		return nil, cloudResponseError(method, "Could not read Cloud API response")
	}
	if len(raw) > limit {
		return nil, cloudResponseError(method, "Cloud API response exceeds the size limit")
	}
	var value map[string]any
	decodeErr := json.Unmarshal(raw, &value)
	if method == http.MethodPost && response.StatusCode >= 500 {
		remote, _ := value["error"].(map[string]any)
		code, validCode := remote["code"].(float64)
		if decodeErr != nil || !validCode || code <= 0 {
			return nil, cloudResponseError(method, fmt.Sprintf("Cloud API HTTP %d without a valid Meta error response", response.StatusCode))
		}
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 || value["error"] != nil {
		code := "cloud_api_error"
		message := cloudRemoteError(response.StatusCode, value)
		if remote, ok := value["error"].(map[string]any); ok && remote["code"] == float64(131047) {
			code = "cloud_window_closed"
			message += ": The customer service window is closed. Wait for a new customer message or call, or explicitly choose an approved template. No automatic retry or template fallback was made"
		}
		return nil, cloudError(code, message)
	}
	if decodeErr != nil || value == nil {
		return nil, cloudResponseError(method, "Cloud API returned invalid JSON")
	}
	return value, nil
}
func cloudRemoteError(status int, value map[string]any) string {
	result := fmt.Sprintf("Cloud API HTTP %d", status)
	if remote, ok := value["error"].(map[string]any); ok {
		if code, ok := remote["code"].(float64); ok {
			result += fmt.Sprintf(" (code %.0f)", code)
		}
		if subcode, ok := remote["error_subcode"].(float64); ok {
			result += fmt.Sprintf(" (subcode %.0f)", subcode)
		}
	}
	// Meta text/details can echo credentials in arbitrary encodings. Expose only
	// numeric diagnostics instead of trying to redact every possible encoding.
	return result
}
func cloudCursor(cursor string) error {
	if len(cursor) > 2048 || strings.IndexFunc(cursor, unicode.IsControl) >= 0 {
		return errors.New("invalid Cloud API cursor")
	}
	return nil
}
func cloudNextCursor(page map[string]any) (*string, error) {
	paging, _ := page["paging"].(map[string]any)
	if _, ok := paging["next"].(string); !ok {
		return nil, nil
	}
	cursors, _ := paging["cursors"].(map[string]any)
	after, ok := cursors["after"].(string)
	if !ok || after == "" {
		return nil, errors.New("Cloud API returned no pagination cursor")
	}
	if err := cloudCursor(after); err != nil {
		return nil, err
	}
	return &after, nil
}
func cloudArray(value any) ([]any, error) {
	items, ok := value.([]any)
	if !ok {
		return nil, errors.New("Cloud API returned an invalid collection")
	}
	return items, nil
}
func (c *cloudClient) collection(ctx context.Context, config Config, path, fields string) ([]any, error) {
	out := []any{}
	after := ""
	for range 100 {
		query := url.Values{"limit": {"100"}}
		if fields != "" {
			query.Set("fields", fields)
		}
		if after != "" {
			query.Set("after", after)
		}
		page, err := c.request(ctx, config, http.MethodGet, path, query, nil)
		if err != nil {
			return nil, err
		}
		items, err := cloudArray(page["data"])
		if err != nil {
			return nil, err
		}
		out = append(out, items...)
		next, err := cloudNextCursor(page)
		if err != nil {
			return nil, err
		}
		if next == nil {
			return out, nil
		}
		if *next == after {
			return nil, errors.New("Cloud API returned a repeated pagination cursor")
		}
		after = *next
	}
	return nil, errors.New("Cloud API collection exceeds the page limit")
}
func (c *cloudClient) checkConnection(ctx context.Context, config Config) (any, error) {
	phone, err := c.request(ctx, config, http.MethodGet, config.Cloud.PhoneNumberID, url.Values{"fields": {"id,display_phone_number,verified_name,quality_rating,code_verification_status"}}, nil)
	if err != nil {
		return nil, err
	}
	if phone["id"] != config.Cloud.PhoneNumberID {
		return nil, errors.New("Cloud API returned an unexpected phone number")
	}
	for _, key := range []string{"display_phone_number", "verified_name"} {
		if _, ok := phone[key].(string); !ok {
			return nil, errors.New("Cloud API returned invalid phone details")
		}
	}
	phones, err := c.collection(ctx, config, config.Cloud.BusinessAccountID+"/phone_numbers", "id")
	if err != nil {
		return nil, err
	}
	inBusiness := false
	for _, item := range phones {
		p, _ := item.(map[string]any)
		if p["id"] == config.Cloud.PhoneNumberID {
			inBusiness = true
		}
	}
	apps, err := c.collection(ctx, config, config.Cloud.BusinessAccountID+"/subscribed_apps", "")
	if err != nil {
		return nil, err
	}
	subscribed := []map[string]any{}
	for _, item := range apps {
		app, _ := item.(map[string]any)
		details, _ := app["whatsapp_business_api_data"].(map[string]any)
		id, ok := details["id"].(string)
		name, named := details["name"].(string)
		if !ok || !named {
			return nil, errors.New("Cloud API returned invalid subscribed app details")
		}
		subscribed = append(subscribed, map[string]any{"id": id, "name": name})
	}
	cleanPhone := map[string]any{}
	for _, key := range []string{"id", "display_phone_number", "verified_name", "quality_rating", "code_verification_status"} {
		cleanPhone[key] = phone[key]
	}
	return map[string]any{"phone": cleanPhone, "phone_in_business_account": inBusiness, "subscribed_apps": subscribed}, nil
}

type CloudTemplatesRequest struct {
	Limit int    `json:"limit"`
	After string `json:"after"`
}
type CloudTextRequest struct {
	ChatID     string `json:"chat_id"`
	Text       string `json:"text"`
	PreviewURL bool   `json:"preview_url"`
}
type CloudTemplateRequest struct {
	ChatID     string           `json:"chat_id"`
	Name       string           `json:"name"`
	Language   string           `json:"language"`
	Components []map[string]any `json:"components"`
}
type CloudSentMessage struct {
	MessageID     string  `json:"message_id"`
	ChatID        string  `json:"chat_id"`
	Status        string  `json:"status"`
	MessageStatus *string `json:"message_status"`
}

func cloudRecipient(chat string) (map[string]any, error) {
	id, err := normalizeCloudChatID(chat)
	if err != nil {
		return nil, err
	}
	body := map[string]any{"messaging_product": "whatsapp", "recipient_type": "individual"}
	if strings.HasPrefix(id, "+") {
		body["to"] = id
	} else {
		body["recipient"] = id
	}
	return body, nil
}
func cloudTextPayload(req CloudTextRequest) (map[string]any, error) {
	if strings.TrimSpace(req.Text) == "" || utf8.RuneCountInString(req.Text) > 4096 {
		return nil, errors.New("text must contain 1..4096 characters")
	}
	body, err := cloudRecipient(req.ChatID)
	if err != nil {
		return nil, err
	}
	body["type"] = "text"
	body["text"] = map[string]any{"body": req.Text, "preview_url": req.PreviewURL}
	return body, nil
}

var cloudTemplateNamePattern = regexp.MustCompile(`^[a-z0-9_]{1,512}$`)
var cloudLanguagePattern = regexp.MustCompile(`^[A-Za-z0-9_]{1,32}$`)

func cloudTemplatePayload(req CloudTemplateRequest) (map[string]any, error) {
	if !cloudTemplateNamePattern.MatchString(req.Name) {
		return nil, errors.New("template name must contain lowercase letters, digits or underscores, at most 512 characters")
	}
	if !cloudLanguagePattern.MatchString(req.Language) {
		return nil, errors.New("invalid template language")
	}
	if req.Components == nil {
		req.Components = []map[string]any{}
	}
	for _, part := range req.Components {
		if part == nil {
			return nil, errors.New("template components must be objects")
		}
	}
	raw, err := json.Marshal(req.Components)
	if err != nil || len(raw) > 65536 {
		return nil, errors.New("template components exceed 65536 bytes")
	}
	body, err := cloudRecipient(req.ChatID)
	if err != nil {
		return nil, err
	}
	body["type"] = "template"
	body["template"] = map[string]any{"name": req.Name, "language": map[string]any{"code": req.Language}, "components": req.Components}
	return body, nil
}
func (c *cloudClient) send(ctx context.Context, config Config, chat string, body map[string]any) (CloudSentMessage, error) {
	result := CloudSentMessage{}
	response, err := c.request(ctx, config, http.MethodPost, config.Cloud.PhoneNumberID+"/messages", nil, body)
	if err != nil {
		return result, err
	}
	messages, err := cloudArray(response["messages"])
	if err != nil || len(messages) == 0 {
		return result, cloudResponseError(http.MethodPost, "Cloud API returned no message acknowledgement")
	}
	message, _ := messages[0].(map[string]any)
	id, _ := message["id"].(string)
	if id == "" || len(id) > 512 || strings.IndexFunc(id, unicode.IsControl) >= 0 || cloudEchoesSecret(config, id) {
		return result, cloudResponseError(http.MethodPost, "Cloud API returned no valid message ID")
	}
	result.MessageID = id
	result.ChatID, _ = normalizeCloudChatID(chat)
	result.Status = "accepted"
	if status, ok := message["message_status"].(string); ok && !cloudEchoesSecret(config, status) {
		result.MessageStatus = &status
	}
	return result, nil
}

// Opaque IDs can contain a short, user-chosen verify token by coincidence.
// Reject a complete credential echo, never an incidental substring of an ID.
func cloudEchoesSecret(config Config, value string) bool {
	for _, secret := range []string{config.Cloud.AccessToken, config.Cloud.AppSecret, config.Cloud.VerifyToken} {
		if secret != "" && value == secret {
			return true
		}
	}
	return false
}
func decodeCloudRequest(input map[string]any, out any) error {
	copy := make(map[string]any, len(input))
	for k, v := range input {
		if k != "_caller_worker_id" {
			copy[k] = v
		}
	}
	raw, err := json.Marshal(copy)
	if err != nil {
		return err
	}
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.DisallowUnknownFields()
	if err = decoder.Decode(out); err != nil {
		return errors.New("invalid function request")
	}
	return nil
}
func outwardCloudMessage(m *MessageView) {
	m.ChatID = strings.TrimPrefix(m.ChatID, "cloud:")
	m.SenderID = strings.TrimPrefix(m.SenderID, "cloud:")
}
func retainCloudSend(inbox *Inbox, sent CloudSentMessage, kind, text string, requestedAt time.Time) error {
	chat, _ := cloudStorageChatID(sent.ChatID)
	status := "submitted"
	// Provisional until the sent webhook supplies Meta's timestamp.
	_, err := inbox.RecordExternal([]MessageView{{MessageID: sent.MessageID, ChatID: chat, SenderID: "me", FromMe: true, TimestampMS: requestedAt.UnixMilli(), Kind: kind, Text: text, Source: "sent", Status: &status}}, nil)
	if err != nil {
		return errors.New("Cloud API accepted the message, but local archival failed; do not resend blindly")
	}
	return nil
}
func registerCloud(r *Registry) {
	client := newCloudClient()
	registerCloudDocuments(r, client)
	r.Add("whatsapp::cloud::status", func(ctx context.Context, input map[string]any) (any, error) {
		var req struct{}
		if err := decodeCloudRequest(input, &req); err != nil {
			return nil, err
		}
		return cloudStatus(r.Manager.Config()), nil
	})
	r.Add("whatsapp::cloud::check_connection", func(ctx context.Context, input map[string]any) (any, error) {
		var req struct{}
		if err := decodeCloudRequest(input, &req); err != nil {
			return nil, err
		}
		return client.checkConnection(ctx, r.Manager.Config())
	})
	r.Add("whatsapp::cloud::templates::list", func(ctx context.Context, input map[string]any) (any, error) {
		req := CloudTemplatesRequest{Limit: 50}
		if err := decodeCloudRequest(input, &req); err != nil {
			return nil, err
		}
		if req.Limit < 1 || req.Limit > 100 {
			return nil, errors.New("limit must be 1..100")
		}
		if err := cloudCursor(req.After); err != nil {
			return nil, err
		}
		query := url.Values{"fields": {"id,name,language,status,category,components,parameter_format"}, "limit": {strconv.Itoa(req.Limit)}}
		if req.After != "" {
			query.Set("after", req.After)
		}
		config := r.Manager.Config()
		page, err := client.request(ctx, config, http.MethodGet, config.Cloud.BusinessAccountID+"/message_templates", query, nil)
		if err != nil {
			return nil, err
		}
		items, err := cloudArray(page["data"])
		if err != nil {
			return nil, err
		}
		templates := make([]map[string]any, 0, len(items))
		for _, item := range items {
			template, ok := item.(map[string]any)
			if !ok {
				return nil, errors.New("Cloud API returned invalid templates")
			}
			clean := map[string]any{}
			for _, key := range []string{"id", "name", "language", "status", "category"} {
				value, ok := template[key].(string)
				if !ok {
					return nil, errors.New("Cloud API returned invalid templates")
				}
				clean[key] = value
			}
			clean["components"] = template["components"]
			if clean["components"] == nil {
				clean["components"] = []any{}
			}
			clean["parameter_format"] = template["parameter_format"]
			templates = append(templates, clean)
		}
		cursor, err := cloudNextCursor(page)
		return map[string]any{"templates": templates, "next_cursor": cursor}, err
	})
	r.Add("whatsapp::cloud::messages::send_text", func(ctx context.Context, input map[string]any) (any, error) {
		var req CloudTextRequest
		if err := decodeCloudRequest(input, &req); err != nil {
			return nil, err
		}
		body, err := cloudTextPayload(req)
		if err != nil {
			return nil, err
		}
		config := r.Manager.Config()
		inbox, err := r.Manager.CloudInbox(config)
		if err != nil {
			return nil, err
		}
		requestedAt := time.Now()
		sent, err := client.send(ctx, config, req.ChatID, body)
		if err != nil {
			return nil, err
		}
		if err = retainCloudSend(inbox, sent, "text", req.Text, requestedAt); err != nil {
			return nil, err
		}
		return sent, nil
	})
	r.Add("whatsapp::cloud::messages::send_template", func(ctx context.Context, input map[string]any) (any, error) {
		var req CloudTemplateRequest
		if err := decodeCloudRequest(input, &req); err != nil {
			return nil, err
		}
		body, err := cloudTemplatePayload(req)
		if err != nil {
			return nil, err
		}
		config := r.Manager.Config()
		inbox, err := r.Manager.CloudInbox(config)
		if err != nil {
			return nil, err
		}
		requestedAt := time.Now()
		sent, err := client.send(ctx, config, req.ChatID, body)
		if err != nil {
			return nil, err
		}
		if err = retainCloudSend(inbox, sent, "template", fmt.Sprintf("[Template: %s (%s)]", req.Name, req.Language), requestedAt); err != nil {
			return nil, err
		}
		return sent, nil
	})
	r.Add("whatsapp::cloud::chats::list", func(ctx context.Context, input map[string]any) (any, error) {
		req := struct {
			Query  string `json:"query"`
			Cursor string `json:"cursor"`
			Limit  int    `json:"limit"`
		}{Limit: 100}
		if err := decodeCloudRequest(input, &req); err != nil {
			return nil, err
		}
		inbox, err := r.Manager.CloudInbox(r.Manager.Config())
		if err != nil {
			return nil, err
		}
		page, err := inbox.ListChats(req.Query, req.Cursor, req.Limit)
		for n := range page.Chats {
			chat := &page.Chats[n]
			if chat.Title == chat.ChatID {
				chat.Title = strings.TrimPrefix(chat.Title, "cloud:")
			}
			chat.ChatID = strings.TrimPrefix(chat.ChatID, "cloud:")
			if chat.LastMessage != nil {
				outwardCloudMessage(chat.LastMessage)
			}
		}
		return page, err
	})
	r.Add("whatsapp::cloud::messages::list", func(ctx context.Context, input map[string]any) (any, error) {
		req := struct {
			ChatID string `json:"chat_id"`
			Cursor string `json:"cursor"`
			Limit  int    `json:"limit"`
		}{Limit: 50}
		if err := decodeCloudRequest(input, &req); err != nil {
			return nil, err
		}
		chat, err := cloudStorageChatID(req.ChatID)
		if err != nil {
			return nil, err
		}
		inbox, err := r.Manager.CloudInbox(r.Manager.Config())
		if err != nil {
			return nil, err
		}
		page, err := inbox.ListMessages(chat, req.Cursor, req.Limit)
		for n := range page.Messages {
			outwardCloudMessage(&page.Messages[n])
		}
		return page, err
	})
}
