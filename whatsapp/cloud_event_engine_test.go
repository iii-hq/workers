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
	validate "github.com/santhosh-tekuri/jsonschema/v6"
)

// Exercise the HTTP worker, Cloud archive and whatsapp::cloud::on-events
// observers together. The surrounding smoke test owns the isolated engine and
// synthetic Cloud account.
func smokeCloudEvents(t *testing.T, engine, engineURL string, port int) {
	t.Helper()
	const callbackID = "go-smoke::cloud-event"
	const callbackNamespace = "whatsapp-go-cloud-consumer"
	const chatID = "+15550000009"
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	registrar := iii.New(engineURL, iii.WithName("go-smoke-cloud-observers"), iii.WithNamespace(smokeNamespace))
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
		payload json.RawMessage
		err     error
	}
	received := make(chan callback, 64)
	if _, err := consumer.RegisterFunction(callbackID, func(callCtx context.Context, raw json.RawMessage) (any, error) {
		var result callback
		result.payload = raw
		result.err = json.Unmarshal(raw, &result.event)
		metadata, ok := iii.MetadataFromContext(callCtx)
		var route struct {
			Binding string `json:"binding"`
		}
		if !ok || json.Unmarshal(metadata, &route) != nil || route.Binding == "" {
			result.err = fmt.Errorf("observer metadata was lost: %s", metadata)
		}
		result.binding = route.Binding
		if result.err == nil && result.event.Kind == "message.received" {
			// Query from inside the target: delaying this until after receipt
			// would fail to catch publication before the archive transaction.
			query, _ := json.Marshal(map[string]string{"chat_id": result.event.ChatID})
			archiveRaw, err := consumer.Call(callCtx, iii.CallRequest{FunctionID: "whatsapp::cloud::messages::list", Namespace: smokeNamespace, Payload: query, Timeout: 4 * time.Second})
			if err != nil {
				result.err = fmt.Errorf("target archive read: %w", err)
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
					result.err = fmt.Errorf("event %s reached its target before archive visibility", result.event.MessageID)
				}
			}
		}
		received <- result
		return map[string]bool{"ok": result.err == nil}, nil
	}); err != nil {
		t.Fatal(err)
	}
	// A target with the same ID in the provider namespace must never run.
	if _, err := registrar.RegisterFunction(callbackID, func(context.Context, json.RawMessage) (any, error) {
		received <- callback{err: fmt.Errorf("Cloud event target resolved in the provider namespace")}
		return nil, nil
	}); err != nil {
		t.Fatal(err)
	}
	smokeWait(t, "Cloud event target", func() bool {
		_, err := smokeCall(engine, engineURL, "default", "engine::functions::info", map[string]any{"function_id": callbackID, "namespace": callbackNamespace})
		return err == nil
	})
	// Observe with the source example discovered through the same engine
	// function used by ADE. Only the target and its metadata are the test's.
	info, err := smokeCall(engine, engineURL, "default", "engine::functions::info", map[string]any{"function_id": "whatsapp::cloud::on-events", "namespace": smokeNamespace})
	if err != nil {
		t.Fatal(err)
	}
	if info["private"] != true {
		t.Fatalf("whatsapp::cloud::on-events must be private: %v", info)
	}
	if _, takesArgs := info["args_format"]; takesArgs {
		t.Fatalf("whatsapp::cloud::on-events takes no args: %v", info)
	}
	// Validate using the schema actually advertised through the locally linked
	// SDK, including every event received below, not only the source files.
	eventSchema := compileDiscoveredCloudSchema(t, info["response_schema"])
	description, _ := info["description"].(string)
	_, example, found := strings.Cut(description, "```json\n")
	if !found {
		t.Fatal("Cloud event discovery has no JSON observe example")
	}
	example, _, found = strings.Cut(example, "\n```")
	if !found {
		t.Fatal("Cloud event discovery has an incomplete observe example")
	}
	var source struct {
		FunctionID string         `json:"function_id"`
		Namespace  string         `json:"namespace"`
		Filters    map[string]any `json:"filters"`
		Deliver    iii.Deliver    `json:"deliver"`
	}
	if err := json.Unmarshal([]byte(example), &source); err != nil {
		t.Fatal(err)
	}
	if source.FunctionID != "whatsapp::cloud::on-events" || source.Namespace != smokeNamespace {
		t.Fatalf("Cloud guide did not resolve the provider namespace: %+v", source)
	}
	// The target resolves in the observing worker's namespace, which differs
	// from the provider's in the discovered guide.
	guideObserver := iii.New(engineURL, iii.WithName("go-smoke-cloud-guide"), iii.WithNamespace(callbackNamespace))
	if err := guideObserver.Connect(ctx); err != nil {
		t.Fatal(err)
	}
	defer guideObserver.Close()
	// Connect can finish before the SDK announces worker metadata. The engine
	// must know this connection's namespace before resolving the target.
	smokeWait(t, "Cloud guide observer namespace", func() bool {
		result, err := smokeCall(engine, engineURL, "default", "engine::workers::list", map[string]any{})
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
	discovery, err := guideObserver.Observe(iii.Source{FunctionID: source.FunctionID, Namespace: source.Namespace, Filters: source.Filters, Deliver: source.Deliver}, callbackID, iii.ObserverOptions{Metadata: json.RawMessage(`{"binding":"discovery"}`)})
	if err != nil {
		t.Fatal(err)
	}
	filters := map[string]iii.Filters{
		"matching": {
			"input.kind":                iii.Op{"$eq": "message.received"},
			"input.chat_id":             iii.Op{"$in": []string{"+15550009999", chatID}},
			"input.sender_id":           iii.Op{"$eq": chatID},
			"input.message_type":        iii.Op{"$in": []string{"image", "text"}},
			"input.business_account_id": iii.Op{"$eq": "123"},
			"input.phone_number_id":     iii.Op{"$eq": "456"},
		},
		"received":        {"input.kind": iii.Op{"$eq": "message.received"}, "input.chat_id": iii.Op{"$eq": chatID}},
		"statuses":        {"input.kind": iii.Op{"$eq": "message.status"}, "input.chat_id": iii.Op{"$eq": chatID}, "input.status": iii.Op{"$in": []string{"sent", "delivered", "read"}}},
		"excluded-chat":   {"input.chat_id": iii.Op{"$eq": "+15550009999"}},
		"excluded-sender": {"input.chat_id": iii.Op{"$eq": chatID}, "input.sender_id": iii.Op{"$eq": "+15550009999"}},
		"excluded-type":   {"input.chat_id": iii.Op{"$eq": chatID}, "input.message_type": iii.Op{"$eq": "image"}},
		"excluded-status": {"input.kind": iii.Op{"$eq": "message.status"}, "input.chat_id": iii.Op{"$eq": chatID}, "input.status": iii.Op{"$eq": "failed"}},
		"excluded-waba":   {"input.chat_id": iii.Op{"$eq": chatID}, "input.business_account_id": iii.Op{"$eq": "999"}},
		"excluded-phone":  {"input.chat_id": iii.Op{"$eq": chatID}, "input.phone_number_id": iii.Op{"$eq": "999"}},
	}
	handles := []*iii.ObserverHandle{discovery}
	for name, filter := range filters {
		metadata, _ := json.Marshal(map[string]string{"binding": name})
		handle, err := registrar.Observe(iii.Source{FunctionID: "whatsapp::cloud::on-events", Filters: filter, Deliver: iii.DeliverOutput}, callbackID, iii.ObserverOptions{Metadata: metadata, TargetNamespace: callbackNamespace})
		if err != nil {
			t.Fatal(err)
		}
		handles = append(handles, handle)
	}
	readyCtx, readyCancel := context.WithTimeout(ctx, 15*time.Second)
	defer readyCancel()
	for _, handle := range handles {
		if err := handle.Ready(readyCtx); err != nil {
			t.Fatalf("Cloud event observer %s: %v", handle.ID(), err)
		}
	}
	detail, err := smokeCall(engine, engineURL, "default", "engine::observers::get", map[string]any{"id": discovery.ID()})
	if err != nil {
		t.Fatal(err)
	}
	if target, _ := detail["target"].(map[string]any); target["function_id"] != callbackID || target["namespace"] != callbackNamespace {
		t.Fatalf("discovered observer has the wrong target: %v", detail)
	}
	status, err := smokeCall(engine, engineURL, smokeNamespace, "whatsapp::cloud::status", map[string]any{})
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
	// the queue. Target completion may still arrive in a different order.
	post(message(barrierID))
	expected := map[string]bool{}
	for _, binding := range []string{"matching", "received", "discovery"} {
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
			var value any
			if err := json.Unmarshal(result.payload, &value); err != nil {
				t.Fatal(err)
			}
			if err := eventSchema.Validate(value); err != nil {
				t.Fatalf("event does not satisfy advertised schema: %v", err)
			}
			key := result.binding + "/" + event.MessageID + "/" + event.Status
			if !expected[key] {
				t.Fatalf("unexpected or duplicate Cloud event: %s (%s)", key, event.Kind)
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
			t.Fatalf("missing Cloud events: %v", expected)
		}
	}
	select {
	case extra := <-received:
		t.Fatalf("unexpected extra Cloud event: %+v", extra)
	case <-time.After(250 * time.Millisecond):
	}
}

func compileDiscoveredCloudSchema(t *testing.T, document any) *validate.Schema {
	t.Helper()
	if document == nil {
		t.Fatal("Cloud event discovery did not advertise a schema")
	}
	compiler := validate.NewCompiler()
	const id = "https://whatsapp.test/discovered.schema.json"
	if err := compiler.AddResource(id, document); err != nil {
		t.Fatal(err)
	}
	schema, err := compiler.Compile(id)
	if err != nil {
		t.Fatal(err)
	}
	return schema
}
