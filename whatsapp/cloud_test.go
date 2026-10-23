package main

import (
	"context"
	"crypto/hmac"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"reflect"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/coder/websocket"
	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

func cloudTestConfig() Config {
	c := DefaultConfig()
	c.Cloud.Enabled = true
	c.Cloud.BusinessAccountID = "123"
	c.Cloud.PhoneNumberID = "456"
	c.Cloud.AccessToken = "test-access-secret"
	c.Cloud.AppSecret = "test-app-secret"
	c.Cloud.VerifyToken = "test-verify-secret"
	return c
}
func cloudTestClient(t *testing.T, handler http.HandlerFunc) *cloudClient {
	t.Helper()
	server := httptest.NewServer(handler)
	t.Cleanup(server.Close)
	client := newCloudClient()
	client.base = server.URL
	return client
}
func cloudTestSignature(raw []byte) string {
	mac := hmac.New(sha256.New, []byte(cloudTestConfig().Cloud.AppSecret))
	_, _ = mac.Write(raw)
	return "sha256=" + hex.EncodeToString(mac.Sum(nil))
}
func cloudTestMessage(id string) map[string]any {
	return map[string]any{"id": id, "from": "15551234567", "timestamp": "1700000000", "type": "text", "text": map[string]any{"body": "Local test café"}}
}
func cloudTestChange(messages ...map[string]any) map[string]any {
	return map[string]any{"field": "messages", "value": map[string]any{"messaging_product": "whatsapp", "metadata": map[string]any{"phone_number_id": "456"}, "contacts": []any{map[string]any{"wa_id": "15551234567", "profile": map[string]any{"name": "Test sender"}}}, "messages": messages}}
}
func cloudTestPayload(changes ...map[string]any) map[string]any {
	return map[string]any{"object": "whatsapp_business_account", "entry": []any{map[string]any{"id": "123", "changes": changes}}}
}
func cloudTestParse(t *testing.T, payload any) cloudDeliveryBatch {
	t.Helper()
	raw, _ := json.Marshal(payload)
	batch, err := parseCloudDelivery(cloudTestConfig().Cloud, raw)
	if err != nil {
		t.Fatal(err)
	}
	return batch
}

func TestCloudIdentifiersConfigAndSafeStatus(t *testing.T) {
	for raw, want := range map[string]string{" +1 (202) 555-0123 ": "+12025550123", "5511999999999": "+5511999999999", "BR.ENT.abc123": "BR.ENT.abc123", "US.123Ab": "US.123Ab"} {
		got, err := normalizeCloudChatID(raw)
		if err != nil || got != want {
			t.Fatalf("%q -> %q, %v", raw, got, err)
		}
	}
	for _, raw := range []string{"123@s.whatsapp.net", "123@g.us", "cloud:+15551234567", "../123", "us.abc", "US.", "0012025550123", "+1234567890123456"} {
		if _, err := normalizeCloudChatID(raw); err == nil {
			t.Fatalf("invalid ID accepted: %q", raw)
		}
	}
	c := cloudTestConfig()
	if err := c.Cloud.Validate(); err != nil {
		t.Fatal(err)
	}
	status := cloudStatus(c)
	if !status.Configured || !status.WebhookReady {
		t.Fatalf("bad status: %+v", status)
	}
	raw, _ := json.Marshal(status)
	debug := fmt.Sprintf("%+v %#v", c.Cloud, c.Cloud)
	for _, secret := range []string{c.Cloud.AccessToken, c.Cloud.AppSecret, c.Cloud.VerifyToken} {
		if strings.Contains(string(raw)+debug, secret) {
			t.Fatal("credential exposed")
		}
	}
	redacted := c.Cloud.Redacted()
	if redacted.AccessToken != "" || redacted.AppSecret != "" || redacted.VerifyToken != "" {
		t.Fatal("redaction failed")
	}
	c.Cloud.AccessToken = "bad\nheader"
	if err := c.Cloud.Validate(); err == nil {
		t.Fatal("newline credential accepted")
	}
}
func TestCloudSendPayloadsAndAcknowledgement(t *testing.T) {
	var calls atomic.Int32
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		if r.URL.Path != "/v26.0/456/messages" || r.Method != "POST" {
			t.Errorf("unexpected request: %s %s", r.Method, r.URL)
		}
		if r.Header.Get("Authorization") != "Bearer test-access-secret" {
			t.Error("missing authorization")
		}
		var body map[string]any
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			t.Error(err)
		}
		if body["recipient"] != "BR.abc123" || body["to"] != nil {
			t.Errorf("wrong BSUID recipient: %+v", body)
		}
		_, _ = w.Write([]byte(`{"messages":[{"id":"wamid.test","message_status":"accepted"}]}`))
	})
	body, err := cloudTextPayload(CloudTextRequest{ChatID: "BR.abc123", Text: "Hello"})
	if err != nil {
		t.Fatal(err)
	}
	sent, err := client.send(context.Background(), cloudTestConfig(), "BR.abc123", body)
	if err != nil || sent.MessageID != "wamid.test" || sent.Status != "accepted" || calls.Load() != 1 {
		t.Fatalf("send: %+v, %v (%d)", sent, err, calls.Load())
	}
	if _, err = cloudTextPayload(CloudTextRequest{ChatID: "+15551234567", Text: strings.Repeat("é", 4097)}); err == nil {
		t.Fatal("oversized Unicode text accepted")
	}
	if _, err = cloudTextPayload(CloudTextRequest{ChatID: "+15551234567", Text: " "}); err == nil {
		t.Fatal("empty text accepted")
	}
	if _, err = cloudTemplatePayload(CloudTemplateRequest{ChatID: "+15551234567", Name: "hello_world", Language: "en_US", Components: []map[string]any{nil}}); err == nil {
		t.Fatal("null component accepted")
	}
	payload, err := cloudTemplatePayload(CloudTemplateRequest{ChatID: "+15551234567", Name: "hello_world", Language: "en_US"})
	if err != nil || payload["to"] != "+15551234567" {
		t.Fatalf("template: %+v, %v", payload, err)
	}
}

func TestCloudTextAndTemplateHTTPRemainJSON(t *testing.T) {
	textBody, err := cloudTextPayload(CloudTextRequest{ChatID: "+15551234567", Text: "Existing text transport", PreviewURL: true})
	if err != nil {
		t.Fatal(err)
	}
	templateBody, err := cloudTemplatePayload(CloudTemplateRequest{ChatID: "+15551234567", Name: "hello_world", Language: "en_US"})
	if err != nil {
		t.Fatal(err)
	}
	for _, body := range []map[string]any{textBody, templateBody} {
		t.Run(body["type"].(string), func(t *testing.T) {
			var calls atomic.Int32
			client := cloudTestClient(t, func(w http.ResponseWriter, request *http.Request) {
				calls.Add(1)
				if request.Method != http.MethodPost || request.URL.Path != "/v26.0/456/messages" || request.Header.Get("Content-Type") != "application/json" || request.Header.Get("Authorization") != "Bearer test-access-secret" {
					t.Error("existing Cloud JSON transport changed")
				}
				var received any
				if err := json.NewDecoder(request.Body).Decode(&received); err != nil {
					t.Error(err)
				}
				encoded, _ := json.Marshal(body)
				var expected any
				_ = json.Unmarshal(encoded, &expected)
				if !reflect.DeepEqual(received, expected) {
					t.Error("existing message payload changed")
				}
				_, _ = w.Write([]byte(`{"messages":[{"id":"wamid.existing","message_status":"accepted"}]}`))
			})
			sent, err := client.send(context.Background(), cloudTestConfig(), "+15551234567", body)
			if err != nil || sent.MessageID != "wamid.existing" || sent.Status != "accepted" || calls.Load() != 1 {
				t.Fatalf("existing send transport failed: %v, calls=%d", err, calls.Load())
			}
		})
	}
}
func TestCloudErrorsAreRedactedRedirectsAndRetriesAreDisabled(t *testing.T) {
	var calls atomic.Int32
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		w.WriteHeader(503)
		_, _ = w.Write([]byte(`{"error":{"code":131047,"message":"test-access-secret test-app-secret","error_data":{"details":"test-verify-secret"}}}`))
	})
	_, err := client.request(context.Background(), cloudTestConfig(), http.MethodPost, "456/messages", nil, map[string]any{"text": "test"})
	if err == nil || calls.Load() != 1 {
		t.Fatalf("unexpected retry or success: %v %d", err, calls.Load())
	}
	for _, secret := range []string{"test-access-secret", "test-app-secret", "test-verify-secret"} {
		if strings.Contains(err.Error(), secret) {
			t.Fatal("remote error exposed a credential")
		}
	}
	if !strings.Contains(err.Error(), "131047") {
		t.Fatal("actionable remote code omitted")
	}
	calls.Store(0)
	redirect := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		w.Header().Set("Location", "/unexpected")
		w.WriteHeader(302)
	})
	if _, err = redirect.request(context.Background(), cloudTestConfig(), http.MethodGet, "456", nil, nil); err == nil || calls.Load() != 1 {
		t.Fatal("redirect followed")
	}
}
func TestCloudResponseBoundsAndPaginationStayOnFixedHost(t *testing.T) {
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte(strings.Repeat("x", 2*1024*1024+1)))
	})
	if _, err := client.request(context.Background(), cloudTestConfig(), http.MethodGet, "456", nil, nil); err == nil || !strings.Contains(err.Error(), "size limit") {
		t.Fatalf("oversized response: %v", err)
	}
	var calls atomic.Int32
	client = cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		if r.URL.Query().Get("after") == "" {
			_, _ = w.Write([]byte(`{"data":[{"id":"a"}],"paging":{"next":"https://attacker.invalid/?access_token=secret","cursors":{"after":"page-two"}}}`))
		} else {
			if r.URL.Query().Get("after") != "page-two" {
				t.Error("invalid after cursor")
			}
			_, _ = w.Write([]byte(`{"data":[{"id":"b"}]}`))
		}
	})
	items, err := client.collection(context.Background(), cloudTestConfig(), "123/phone_numbers", "id")
	if err != nil || len(items) != 2 || calls.Load() != 2 {
		t.Fatalf("pagination: %+v %v", items, err)
	}
}
func TestCloudWebhookSignatureChallengeAndAccountIsolation(t *testing.T) {
	config := cloudTestConfig().Cloud
	query := map[string]string{"hub.mode": "subscribe", "hub.verify_token": config.VerifyToken, "hub.challenge": "001234"}
	response := verifyCloudWebhook(config, query)
	if response.StatusCode != 200 || response.Body != "001234" || !strings.HasPrefix(response.Headers["content-type"], "text/plain") {
		t.Fatalf("challenge: %+v", response)
	}
	query["hub.verify_token"] = "wrong"
	if verifyCloudWebhook(config, query).StatusCode != 403 {
		t.Fatal("bad token accepted")
	}
	raw := []byte("{ \"object\": \"whatsapp_business_account\",\r\n \"entry\": [] } \n")
	signed := cloudTestSignature(raw)
	if !verifyCloudSignature(config.AppSecret, raw, signed) {
		t.Fatal("valid raw signature rejected")
	}
	var projected any
	_ = json.Unmarshal(raw, &projected)
	compact, _ := json.Marshal(projected)
	if verifyCloudSignature(config.AppSecret, compact, signed) {
		t.Fatal("reserialized bytes authenticated")
	}
	payload := cloudTestPayload(cloudTestChange(cloudTestMessage("one"), cloudTestMessage("two")), cloudTestChange(cloudTestMessage("three")))
	entries := payload["entry"].([]any)
	wrongNumber := cloudTestChange(cloudTestMessage("wrong-number"))
	wrongNumber["value"].(map[string]any)["metadata"] = map[string]any{"phone_number_id": "999"}
	payload["entry"] = append(entries, map[string]any{"id": "999", "changes": []any{cloudTestChange(cloudTestMessage("wrong-account"))}}, map[string]any{"id": "123", "changes": []any{wrongNumber}})
	batch := cloudTestParse(t, payload)
	if len(batch.messages) != 3 || batch.messages[0].SenderName == nil || *batch.messages[0].SenderName != "Test sender" {
		t.Fatalf("wrong batch: %+v", batch)
	}
	private := cloudTestMessage("bsuid")
	delete(private, "from")
	private["from_user_id"] = "BR.ENT.private123"
	change := cloudTestChange(private)
	change["value"].(map[string]any)["statuses"] = []any{map[string]any{"id": "sent", "status": "read", "timestamp": "1700000001", "recipient_user_id": "BR.ENT.private123"}}
	batch = cloudTestParse(t, cloudTestPayload(change))
	if batch.messages[0].ChatID != "cloud:BR.ENT.private123" || batch.receipts[0].ChatID != batch.messages[0].ChatID {
		t.Fatal("BSUID was not preserved")
	}
}
func TestCloudWebhookRawSDKChannelAndDuplicatePersistence(t *testing.T) {
	config := cloudTestConfig()
	config.DataDir = t.TempDir()
	manager, err := newTestManager(config)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = manager.Shutdown(context.Background()) })
	payload, _ := json.MarshalIndent(cloudTestPayload(cloudTestChange(cloudTestMessage("one"))), "", "  ")
	raw := append(payload, []byte(" \r\n")...)
	var streamCalls atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		streamCalls.Add(1)
		if r.URL.Path != "/ws/channels/test-channel" || r.URL.Query().Get("key") != "test-key" || r.URL.Query().Get("dir") != "read" {
			t.Errorf("invalid SDK channel path: %s", r.URL)
		}
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			t.Error(err)
			return
		}
		defer conn.CloseNow()
		ctx, cancel := context.WithTimeout(r.Context(), 3*time.Second)
		defer cancel()
		if err = conn.Write(ctx, websocket.MessageBinary, raw[:17]); err == nil {
			err = conn.Write(ctx, websocket.MessageBinary, raw[17:])
		}
		if err != nil {
			t.Error(err)
			return
		}
		_ = conn.Close(websocket.StatusNormalClosure, "done")
	}))
	defer server.Close()
	engineURL := "ws" + strings.TrimPrefix(server.URL, "http")
	ref := &iii.StreamChannelRef{ChannelID: "test-channel", AccessKey: "test-key", Direction: iii.ChannelRead}
	req := cloudReceiveRequest{Headers: map[string]string{"X-Hub-Signature-256": cloudTestSignature(raw)}, RequestBody: ref}
	for range 2 {
		response := receiveCloudWebhook(context.Background(), manager, engineURL, req)
		if response.StatusCode != 200 {
			t.Fatalf("valid delivery failed: %+v", response)
		}
	}
	inbox, err := manager.CloudInbox(config)
	if err != nil {
		t.Fatal(err)
	}
	page, err := inbox.ListMessages("cloud:+15551234567", "", 10)
	if err != nil || len(page.Messages) != 1 || page.Messages[0].Text != "Local test café" {
		t.Fatalf("delivery not persisted: %+v, %v", page, err)
	}
	if len(manager.CloudEventQueue) != 1 {
		t.Fatalf("duplicate webhook published %d events", len(manager.CloudEventQueue))
	}
	event := <-manager.CloudEventQueue
	if event.Kind != "message.received" || event.MessageID != page.Messages[0].MessageID || event.ChatID != "+15551234567" {
		t.Fatalf("wrong webhook event: %+v", event)
	}
	req.Headers["X-Hub-Signature-256"] = "sha256=" + strings.Repeat("0", 64)
	if receiveCloudWebhook(context.Background(), manager, engineURL, req).StatusCode != 403 {
		t.Fatal("bad signature accepted")
	}
	req.RequestBody = nil
	if receiveCloudWebhook(context.Background(), manager, engineURL, req).StatusCode != 400 {
		t.Fatal("missing original body accepted")
	}
	if streamCalls.Load() != 3 {
		t.Fatalf("unexpected channel calls: %d", streamCalls.Load())
	}
	if len(manager.CloudEventQueue) != 0 {
		t.Fatal("rejected webhook published an event")
	}
}

func TestCloudSentReceiptCorrectsArchiveTimeWithoutRegressingStatus(t *testing.T) {
	inbox := inboxTestOpen(t, 10)
	message := inboxTestMessage("outgoing", 1700000018000)
	message.ChatID = "cloud:+15551234567"
	message.FromMe = true
	submitted := "submitted"
	message.Status = &submitted
	if _, err := inbox.RecordExternal([]MessageView{message}, nil); err != nil {
		t.Fatal(err)
	}
	for _, status := range []struct{ state, timestamp string }{{"read", "1700000005"}, {"sent", "1700000000"}, {"delivered", "1700000003"}, {"sent", "1700000000"}} {
		change := cloudTestChange()
		delete(change["value"].(map[string]any), "messages")
		change["value"].(map[string]any)["statuses"] = []any{map[string]any{"id": message.MessageID, "status": status.state, "timestamp": status.timestamp, "recipient_id": "15551234567"}}
		batch := cloudTestParse(t, cloudTestPayload(change))
		if _, err := inbox.RecordExternal(batch.messages, batch.receipts); err != nil {
			t.Fatal(err)
		}
	}
	page, err := inbox.ListMessages(message.ChatID, "", 10)
	if err != nil || len(page.Messages) != 1 {
		t.Fatalf("message missing: %+v, %v", page, err)
	}
	got := page.Messages[0]
	if got.TimestampMS != 1700000000000 || got.Status == nil || *got.Status != "read" {
		t.Fatalf("sent time must replace the local clock without regressing read: %+v", got)
	}
}

func TestCloudWebhookValidatesSentTimestampsAndKeepsOtherStatusTimesSeparate(t *testing.T) {
	for _, timestamp := range []any{nil, "", "invalid", "-1", "9223372036854776", "9223372036854775808", float64(1700000000)} {
		t.Run(fmt.Sprint(timestamp), func(t *testing.T) {
			change := cloudTestChange(cloudTestMessage("incoming"))
			change["value"].(map[string]any)["statuses"] = []any{map[string]any{"id": "outgoing", "status": "sent", "timestamp": timestamp, "recipient_id": "15551234567"}}
			raw, err := json.Marshal(cloudTestPayload(change))
			if err != nil {
				t.Fatal(err)
			}
			if _, err = parseCloudDelivery(cloudTestConfig().Cloud, raw); err == nil {
				t.Fatal("invalid sent timestamp accepted")
			}
		})
	}
	for _, state := range []string{"sent", "delivered", "read", "failed"} {
		change := cloudTestChange(cloudTestMessage("incoming"))
		change["value"].(map[string]any)["statuses"] = []any{map[string]any{"id": "outgoing", "status": state, "timestamp": "0", "recipient_id": "15551234567"}}
		batch := cloudTestParse(t, cloudTestPayload(change))
		if len(batch.receipts) != 1 || (batch.receipts[0].SentAtMS != nil) != (state == "sent") {
			t.Fatalf("status time misclassified for %s: %+v", state, batch.receipts)
		}
		if state == "sent" && *batch.receipts[0].SentAtMS != 0 {
			t.Fatal("epoch timestamp was not preserved")
		}
	}
}
