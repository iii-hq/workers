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

func TestCloudEventConfigSchemaMatchesFilter(t *testing.T) {
	schema := compileCloudEventSchema(t, cloudEventConfigSchema)
	cases := []struct {
		name, raw string
		valid     bool
	}{
		{"defaults", `{}`, true},
		{"empty lists", `{"kinds":[],"chat_ids":[],"sender_ids":[],"message_types":[],"statuses":[]}`, true},
		{"null filters", `{"kinds":null,"chat_ids":null,"sender_ids":null,"message_types":null,"statuses":null,"business_account_id":null,"phone_number_id":null}`, true},
		{"empty identifiers", `{"business_account_id":"","phone_number_id":""}`, true},
		{"formatted identities", `{"chat_ids":["+1 (555) 123-4567","15551234567","\tBR.ENT.customer42\n","\u2000+15551234567\u3000"],"sender_ids":["BR.customer42"]}`, true},
		{"future types", `{"message_types":["interactive","contacts","future_type_2"]}`, true},
		{"status kinds", `{"kinds":["message.received","message.status"],"statuses":["sent","delivered","read","failed"],"business_account_id":"123","phone_number_id":"456"}`, true},
		{"status with default kind", `{"statuses":["read"]}`, true},
		{"unknown filter", `{"sender_id":"15551234567"}`, false},
		{"nonobject", `[]`, false},
		{"null config", `null`, false},
		{"wrong list type", `{"chat_ids":"15551234567"}`, false},
		{"null list item", `{"chat_ids":[null]}`, false},
		{"unknown kind", `{"kinds":["message"]}`, false},
		{"unknown status", `{"statuses":["submitted"]}`, false},
		{"invalid message type", `{"message_types":["Text"]}`, false},
		{"empty message type", `{"message_types":[""]}`, false},
		{"invalid account", `{"business_account_id":"+123"}`, false},
		{"numeric phone ID", `{"phone_number_id":123}`, false},
		{"long phone ID", `{"phone_number_id":"123456789012345678901234567890123"}`, false},
	}
	for _, test := range cases {
		t.Run(test.name, func(t *testing.T) {
			var value any
			if err := json.Unmarshal([]byte(test.raw), &value); err != nil {
				t.Fatal(err)
			}
			_, parseErr := parseCloudEventFilter(json.RawMessage(test.raw))
			schemaErr := schema.Validate(value)
			if (parseErr == nil) != test.valid || (schemaErr == nil) != test.valid {
				t.Fatalf("valid = %t; parser: %v; schema: %v", test.valid, parseErr, schemaErr)
			}
		})
	}
	for key, item := range map[string]string{"kinds": "message.received", "chat_ids": "+15551234567", "sender_ids": "+15551234567", "message_types": "text", "statuses": "read"} {
		t.Run(key+" list bound", func(t *testing.T) {
			for _, count := range []int{128, 129} {
				values := make([]string, count)
				for i := range values {
					values[i] = item
				}
				raw, err := json.Marshal(map[string]any{key: values})
				if err != nil {
					t.Fatal(err)
				}
				var value any
				if err := json.Unmarshal(raw, &value); err != nil {
					t.Fatal(err)
				}
				_, parseErr := parseCloudEventFilter(raw)
				schemaErr := schema.Validate(value)
				if (parseErr == nil) != (count == 128) || (schemaErr == nil) != (count == 128) {
					t.Fatalf("count = %d; parser: %v; schema: %v", count, parseErr, schemaErr)
				}
			}
		})
	}
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
