package main

import (
	"encoding/json"
	"strings"
	"testing"

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
