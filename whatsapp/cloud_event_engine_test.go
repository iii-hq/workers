package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"strings"
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

// Exercise the HTTP worker, Cloud archive and trigger router together. The
// surrounding smoke test owns the isolated engine and synthetic Cloud account.
func smokeCloudEvents(t *testing.T, engine, engineURL string, port int) {
	t.Helper()
	const callbackID = "go-smoke::cloud-event"
	const callbackNamespace = "whatsapp-go-cloud-consumer"
	const chatID = "+15550000009"
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	registrar := iii.New(engineURL, iii.WithName("go-smoke-cloud-bindings"), iii.WithNamespace(smokeNamespace))
	consumer := iii.New(engineURL, iii.WithName("go-smoke-cloud-consumer"), iii.WithNamespace(callbackNamespace))
	defer func() { _ = registrar.Close(); _ = consumer.Close() }()
	if err := registrar.Connect(ctx); err != nil {
		t.Fatal(err)
	}
	if err := consumer.Connect(ctx); err != nil {
		t.Fatal(err)
	}
	type eventPayload struct {
		Kind              string         `json:"kind"`
		BusinessAccountID string         `json:"business_account_id"`
		PhoneNumberID     string         `json:"phone_number_id"`
		MessageID         string         `json:"message_id"`
		ChatID            string         `json:"chat_id"`
		SenderID          string         `json:"sender_id"`
		MessageType       string         `json:"message_type"`
		Text              string         `json:"text"`
		Status            string         `json:"status"`
		TimestampMS       int64          `json:"timestamp_ms"`
		Data              map[string]any `json:"data"`
	}
	type callback struct {
		binding string
		event   eventPayload
		err     error
	}
	received := make(chan callback, 64)
	if err := consumer.RegisterFunction(callbackID, func(callCtx context.Context, raw json.RawMessage) (any, error) {
		var result callback
		result.err = json.Unmarshal(raw, &result.event)
		metadata, ok := iii.MetadataFromContext(callCtx)
		var route struct {
			Binding string `json:"binding"`
		}
		if !ok || json.Unmarshal(metadata, &route) != nil || route.Binding == "" {
			result.err = fmt.Errorf("callback binding metadata was lost: %s", metadata)
		}
		result.binding = route.Binding
		if result.err == nil && result.event.Kind == "message.received" {
			// Query from inside the callback: delaying this until after receipt
			// would fail to catch publication before the archive transaction.
			query, _ := json.Marshal(map[string]string{"chat_id": result.event.ChatID})
			archiveRaw, err := consumer.Trigger(callCtx, iii.TriggerRequest{FunctionID: "whatsapp::cloud::messages::list", Namespace: smokeNamespace, Data: query, Timeout: 4 * time.Second})
			if err != nil {
				result.err = fmt.Errorf("callback archive read: %w", err)
			} else {
				var archive struct {
					Messages []struct {
						MessageID string `json:"message_id"`
						Text      string `json:"text"`
					} `json:"messages"`
				}
				result.err = json.Unmarshal(archiveRaw, &archive)
				found := false
				for _, message := range archive.Messages {
					if message.MessageID == result.event.MessageID && message.Text == result.event.Text {
						found = true
					}
				}
				if !found && result.err == nil {
					result.err = fmt.Errorf("event %s reached its callback before archive visibility", result.event.MessageID)
				}
			}
		}
		received <- result
		return map[string]bool{"ok": result.err == nil}, nil
	}); err != nil {
		t.Fatal(err)
	}
	// A target with the same ID in the provider namespace must never run.
	if err := registrar.RegisterFunction(callbackID, func(context.Context, json.RawMessage) (any, error) {
		received <- callback{err: fmt.Errorf("Cloud callback resolved in the provider namespace")}
		return nil, nil
	}); err != nil {
		t.Fatal(err)
	}
	smokeWait(t, "Cloud callback function", func() bool {
		_, err := smokeTrigger(engine, engineURL, "default", "engine::functions::info", map[string]any{"function_id": callbackID, "namespace": callbackNamespace})
		return err == nil
	})
	// Execute the registration example discovered through the same engine
	// function used by ADE. Only substitute the user's callback and metadata.
	info, err := smokeTrigger(engine, engineURL, "default", "engine::triggers::info", map[string]any{"id": "whatsapp::cloud::events", "namespace": smokeNamespace})
	if err != nil {
		t.Fatal(err)
	}
	description, _ := info["description"].(string)
	_, example, found := strings.Cut(description, "```json\n")
	if !found {
		t.Fatal("Cloud trigger discovery has no JSON registration example")
	}
	example, _, found = strings.Cut(example, "\n```")
	if !found {
		t.Fatal("Cloud trigger discovery has an incomplete registration example")
	}
	var registration map[string]any
	if err := json.Unmarshal([]byte(example), &registration); err != nil {
		t.Fatal(err)
	}
	if registration["trigger_namespace"] != smokeNamespace {
		t.Fatalf("Cloud guide did not resolve the provider namespace: %v", registration)
	}
	registration["function_id"] = callbackID
	registration["metadata"].(map[string]any)["binding"] = "discovery"
	registrationJSON, _ := json.Marshal(registration)
	// Function-path subscriptions outlive the registering connection. The
	// callback's namespace differs from the provider's in the discovered guide.
	guideRegistrar := iii.New(engineURL, iii.WithName("go-smoke-cloud-guide"), iii.WithNamespace(callbackNamespace))
	if err := guideRegistrar.Connect(ctx); err != nil {
		t.Fatal(err)
	}
	defer guideRegistrar.Close()
	// Connect can finish before the SDK announces worker metadata. The engine
	// must know this connection's namespace before resolving the callback.
	smokeWait(t, "Cloud guide registrar namespace", func() bool {
		result, err := smokeTrigger(engine, engineURL, "default", "engine::workers::list", map[string]any{})
		if err != nil {
			return false
		}
		workers, _ := result["workers"].([]any)
		for _, row := range workers {
			worker, _ := row.(map[string]any)
			if worker["name"] == "go-smoke-cloud-guide" && worker["namespace"] == callbackNamespace {
				return true
			}
		}
		return false
	})
	registered, err := guideRegistrar.Trigger(ctx, iii.TriggerRequest{FunctionID: "engine::register_trigger", Namespace: "default", Data: registrationJSON, Timeout: 5 * time.Second})
	if err != nil {
		t.Fatal(err)
	}
	var binding struct {
		ID string `json:"id"`
	}
	if err := json.Unmarshal(registered, &binding); err != nil || binding.ID == "" {
		t.Fatalf("guide registration did not return an id: %s (%v)", registered, err)
	}
	t.Cleanup(func() {
		result, err := smokeTrigger(engine, engineURL, "default", "engine::unregister_trigger", map[string]any{"id": binding.ID})
		if err != nil || result["removed"] != true {
			t.Errorf("guide subscription cleanup: %v (%v)", result, err)
		}
	})
	if err := guideRegistrar.Close(); err != nil {
		t.Fatal(err)
	}
	smokeWait(t, "discovered Cloud subscription after registrar disconnect", func() bool {
		detail, err := smokeTrigger(engine, engineURL, "default", "engine::registered-triggers::info", map[string]any{"id": binding.ID})
		if err == nil && detail["status"] == "active" {
			function, _ := detail["function"].(map[string]any)
			if function["namespace"] != callbackNamespace {
				t.Fatalf("discovered subscription has wrong callback detail: %v", function)
			}
		}
		return err == nil && detail["status"] == "active"
	})
	filters := map[string]map[string]any{
		"matching":        {"kinds": []string{"message.received"}, "chat_ids": []string{"+15550009999", chatID}, "sender_ids": []string{chatID}, "message_types": []string{"image", "text"}, "business_account_id": "123", "phone_number_id": "456"},
		"default":         {"chat_ids": []string{chatID}},
		"statuses":        {"kinds": []string{"message.status"}, "chat_ids": []string{chatID}, "statuses": []string{"sent", "delivered", "read"}},
		"excluded-chat":   {"chat_ids": []string{"+15550009999"}},
		"excluded-sender": {"chat_ids": []string{chatID}, "sender_ids": []string{"+15550009999"}},
		"excluded-type":   {"chat_ids": []string{chatID}, "message_types": []string{"image"}},
		"excluded-status": {"kinds": []string{"message.status"}, "chat_ids": []string{chatID}, "statuses": []string{"failed"}},
		"excluded-waba":   {"chat_ids": []string{chatID}, "business_account_id": "999"},
		"excluded-phone":  {"chat_ids": []string{chatID}, "phone_number_id": "999"},
	}
	for name, filter := range filters {
		config, _ := json.Marshal(filter)
		metadata, _ := json.Marshal(map[string]string{"binding": name})
		if err := registrar.RegisterTriggerNamespaced("cloud-smoke-"+name, "whatsapp::cloud::events", callbackID, callbackNamespace, config, metadata); err != nil {
			t.Fatal(err)
		}
	}
	smokeWait(t, "Cloud event bindings", func() bool {
		result, err := smokeTrigger(engine, engineURL, "default", "engine::registered-triggers::list", map[string]any{"trigger_type": "whatsapp::cloud::events", "function_id": callbackID})
		if err != nil {
			return false
		}
		rows, _ := result["registered_triggers"].([]any)
		active := 0
		for _, row := range rows {
			item, _ := row.(map[string]any)
			if item["status"] == "active" {
				active++
			}
		}
		return active == len(filters)+1
	})
	status, err := smokeTrigger(engine, engineURL, smokeNamespace, "whatsapp::cloud::status", map[string]any{})
	if err != nil {
		t.Fatal(err)
	}
	path, _ := status["webhook_path"].(string)
	endpoint := fmt.Sprintf("http://127.0.0.1:%d%s", port, path)
	client := &http.Client{Transport: &http.Transport{Proxy: nil}, Timeout: 12 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	defer client.CloseIdleConnections()
	post := func(change map[string]any) {
		t.Helper()
		raw, err := json.Marshal(cloudTestPayload(change))
		if err != nil {
			t.Fatal(err)
		}
		request, err := http.NewRequestWithContext(ctx, http.MethodPost, endpoint, bytes.NewReader(raw))
		if err != nil {
			t.Fatal(err)
		}
		request.Header.Set("Content-Type", "application/json")
		request.Header.Set("X-Hub-Signature-256", cloudTestSignature(raw))
		response, err := client.Do(request)
		if err != nil {
			t.Fatal(err)
		}
		_, _ = io.Copy(io.Discard, response.Body)
		_ = response.Body.Close()
		if response.StatusCode != http.StatusOK {
			t.Fatalf("Cloud event webhook: HTTP %d", response.StatusCode)
		}
	}
	message := func(id string) map[string]any {
		value := cloudTestMessage(id)
		value["from"] = "15550000009"
		return cloudTestChange(value)
	}
	const firstID = "wamid.cloud-events.first"
	const barrierID = "wamid.cloud-events.barrier"
	const receiptID = "wamid.cloud-events.outgoing"
	post(message(firstID))
	post(message(firstID))
	for _, value := range []string{"delivered", "delivered", "read", "sent", "sent"} {
		change := cloudTestChange()
		body := change["value"].(map[string]any)
		delete(body, "messages")
		body["statuses"] = []any{map[string]any{"id": receiptID, "recipient_id": "15550000009", "status": value, "timestamp": "1700000001"}}
		post(change)
	}
	// The final fresh message follows the duplicate and status deliveries in
	// the queue. Callback completion may still arrive in a different order.
	post(message(barrierID))
	expected := map[string]bool{}
	for _, binding := range []string{"matching", "default", "discovery"} {
		for _, id := range []string{firstID, barrierID} {
			expected[binding+"/"+id+"/"] = true
		}
	}
	for _, value := range []string{"delivered", "read", "sent"} {
		expected["statuses/"+receiptID+"/"+value] = true
	}
	deadline := time.NewTimer(20 * time.Second)
	defer deadline.Stop()
	for len(expected) > 0 {
		select {
		case result := <-received:
			if result.err != nil {
				t.Fatal(result.err)
			}
			event := result.event
			key := result.binding + "/" + event.MessageID + "/" + event.Status
			if !expected[key] {
				t.Fatalf("unexpected or duplicate Cloud callback: %s (%s)", key, event.Kind)
			}
			delete(expected, key)
			if event.BusinessAccountID != "123" || event.PhoneNumberID != "456" || event.ChatID != chatID || event.Data["id"] != event.MessageID {
				t.Fatalf("Cloud envelope lost account, recipient or raw object: %+v", event)
			}
			if result.binding == "statuses" {
				if event.Kind != "message.status" || event.TimestampMS != 1700000001000 || event.Data["status"] != event.Status {
					t.Fatalf("invalid Cloud receipt: %+v", event)
				}
			} else if event.Kind != "message.received" || event.SenderID != chatID || event.MessageType != "text" || event.Text != "Local test café" || event.TimestampMS != 1700000000000 {
				t.Fatalf("invalid Cloud message: %+v", event)
			}
		case <-deadline.C:
			t.Fatalf("missing Cloud callbacks: %v", expected)
		}
	}
	select {
	case extra := <-received:
		t.Fatalf("unexpected extra Cloud callback: %+v", extra)
	case <-time.After(250 * time.Millisecond):
	}
}
