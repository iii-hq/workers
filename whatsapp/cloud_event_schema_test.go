package main

import (
	"context"
	"encoding/json"
	"reflect"
	"strings"
	"testing"
	"time"

	validate "github.com/santhosh-tekuri/jsonschema/v6"
)

func compileCloudEventSchema(t *testing.T, raw []byte) *validate.Schema {
	t.Helper()
	var document any
	if err := json.Unmarshal(raw, &document); err != nil {
		t.Fatal(err)
	}
	compiler := validate.NewCompiler()
	uri := "https://whatsapp.invalid/test-schema.json"
	if err := compiler.AddResource(uri, document); err != nil {
		t.Fatal(err)
	}
	schema, err := compiler.Compile(uri)
	if err != nil {
		t.Fatal(err)
	}
	for _, example := range document.(map[string]any)["examples"].([]any) {
		if err := schema.Validate(example); err != nil {
			t.Fatalf("documented example is invalid: %v", err)
		}
	}
	return schema
}

func TestCloudEventPayloadSchemaMatchesPublishedEvents(t *testing.T) {
	schema := compileCloudEventSchema(t, cloudEventPayloadSchema)
	message := CloudEvent{
		Kind: "message.received", BusinessAccountID: "123", PhoneNumberID: "456", MessageID: "wamid.example",
		ChatID: "+15551234567", SenderID: "+15551234567", MessageType: "text", Text: "Hello", TimestampMS: 1790870400000,
		Data: json.RawMessage(`{"id":"wamid.example","type":"text","text":{"body":"Hello"}}`),
	}
	status := CloudEvent{
		Kind: "message.status", BusinessAccountID: "123", PhoneNumberID: "456", MessageID: "wamid.example",
		ChatID: "BR.ENT.customer42", Status: "read", TimestampMS: 1790870460000,
		Data: json.RawMessage(`{"id":"wamid.example","status":"read","recipient_user_id":"BR.ENT.customer42"}`),
	}
	oversized := message
	oversized.Data, _ = json.Marshal(map[string]string{"body": strings.Repeat("x", 65536)})
	oversized = boundedCloudEvent(oversized)
	if string(oversized.Data) != `{"content_omitted":true}` {
		t.Fatal("expected oversized event to omit raw content")
	}
	for name, event := range map[string]CloudEvent{"incoming message": message, "status": status, "content omitted": oversized} {
		t.Run(name, func(t *testing.T) {
			raw, err := json.Marshal(event)
			if err != nil {
				t.Fatal(err)
			}
			var value map[string]any
			if err := json.Unmarshal(raw, &value); err != nil {
				t.Fatal(err)
			}
			if err := schema.Validate(value); err != nil {
				t.Fatalf("published event rejected: %v", err)
			}
			value["_caller_worker_id"] = "transport-worker-id"
			if err := schema.Validate(value); err != nil {
				t.Fatalf("transport metadata rejected: %v", err)
			}
			for _, required := range []string{"kind", "business_account_id", "phone_number_id", "message_id", "chat_id", "timestamp_ms", "data"} {
				original := value[required]
				delete(value, required)
				if err := schema.Validate(value); err == nil {
					t.Errorf("schema accepted missing %s", required)
				}
				value[required] = original
			}
			value["kind"] = "session.connected"
			if err := schema.Validate(value); err == nil {
				t.Fatal("schema accepted a native event kind")
			}
		})
	}
}

func TestCloudEventFunctionWireSchemas(t *testing.T) {
	peer := newCallbackPeer(t, nil)
	peer.registerWorker(t)
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	if err := registerCloudEvents(ctx, peer.client, &Manager{CloudEventQueue: make(chan CloudEvent)}); err != nil {
		t.Fatal(err)
	}
	var frame map[string]json.RawMessage
	for frame == nil {
		select {
		case received := <-peer.frames:
			if string(received["type"]) == `"registerfunction"` && string(received["id"]) == `"whatsapp::cloud::on-events"` {
				frame = received
			}
		case <-ctx.Done():
			t.Fatal("Cloud event function registration was not sent")
		}
	}
	if string(frame["private"]) != "true" {
		t.Error("whatsapp::cloud::on-events must be private: only this worker calls it")
	}
	if len(frame["args_format"]) != 0 {
		t.Error("whatsapp::cloud::on-events takes no args; observers filter on its input")
	}
	if len(frame["observable"]) != 0 {
		t.Errorf("whatsapp::cloud::on-events must stay observable, got observable: %s", frame["observable"])
	}
	formats := make(map[string]any)
	for _, field := range []string{"request_format", "response_format"} {
		var document map[string]any
		if err := json.Unmarshal(frame[field], &document); err != nil || len(document) == 0 {
			t.Fatalf("Cloud event function sends no typed %s", field)
		}
		formats[field] = document
	}
	if !reflect.DeepEqual(formats["request_format"], formats["response_format"]) {
		t.Fatal("the event source must return the event it is called with")
	}
	var published map[string]any
	if err := json.Unmarshal(cloudEventPayloadSchema, &published); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(formats["response_format"], published) {
		t.Fatal("whatsapp::cloud::on-events must register the published Cloud event schema")
	}
	t.Run("filters select kind and chat", func(t *testing.T) {
		filters := map[string]any{
			"input.kind":    map[string]any{"$eq": "message.received"},
			"input.chat_id": map[string]any{"$in": []string{"+15550000009", "+15550000010"}},
		}
		for _, tc := range []struct {
			event CloudEvent
			match bool
		}{
			{CloudEvent{Kind: "message.received", ChatID: "+15550000009", Data: json.RawMessage(`{}`)}, true},
			{CloudEvent{Kind: "message.received", ChatID: "+15550000010", Data: json.RawMessage(`{}`)}, true},
			{CloudEvent{Kind: "message.status", ChatID: "+15550000009", Data: json.RawMessage(`{}`)}, false},
			{CloudEvent{Kind: "message.received", ChatID: "+15550000011", Data: json.RawMessage(`{}`)}, false},
		} {
			if got := filterSelects(t, filters, tc.event); got != tc.match {
				t.Errorf("%s/%s: selected = %t, want %t", tc.event.Kind, tc.event.ChatID, got, tc.match)
			}
		}
	})
}
