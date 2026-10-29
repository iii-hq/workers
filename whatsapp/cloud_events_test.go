package main

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

func TestCloudEventFiltersSelectEventFields(t *testing.T) {
	base := CloudEvent{Kind: "message.received", ChatID: "+15551234567", SenderID: "+15551234567", MessageType: "text", BusinessAccountID: "123", PhoneNumberID: "456", Data: json.RawMessage(`{}`)}
	filters := map[string]any{
		"input.kind":                map[string]any{"$eq": "message.received"},
		"input.chat_id":             map[string]any{"$in": []string{"+15551234567", "BR.ENT.customer42"}},
		"input.sender_id":           map[string]any{"$in": []string{"+15551234567", "BR.ENT.customer42"}},
		"input.message_type":        map[string]any{"$in": []string{"text", "interactive"}},
		"input.business_account_id": map[string]any{"$eq": "123"},
		"input.phone_number_id":     map[string]any{"$eq": "456"},
	}
	if !filterSelects(t, filters, base) {
		t.Fatal("matching incoming message rejected")
	}
	second := base
	second.ChatID, second.SenderID, second.MessageType = "BR.ENT.customer42", "BR.ENT.customer42", "interactive"
	if !filterSelects(t, filters, second) {
		t.Fatal("$in alternatives must match")
	}
	for name, change := range map[string]func(*CloudEvent){
		"kind":        func(e *CloudEvent) { e.Kind = "message.status" },
		"chat":        func(e *CloudEvent) { e.ChatID = "+15559999999" },
		"sender":      func(e *CloudEvent) { e.SenderID = "+15559999999" },
		"type":        func(e *CloudEvent) { e.MessageType = "image" },
		"account":     func(e *CloudEvent) { e.BusinessAccountID = "999" },
		"phone":       func(e *CloudEvent) { e.PhoneNumberID = "999" },
		"no sender":   func(e *CloudEvent) { e.SenderID = "" },
		"no type":     func(e *CloudEvent) { e.MessageType = "" },
		"no chat":     func(e *CloudEvent) { e.ChatID = "" },
		"no account":  func(e *CloudEvent) { e.BusinessAccountID = "" },
		"no phone ID": func(e *CloudEvent) { e.PhoneNumberID = "" },
	} {
		t.Run(name, func(t *testing.T) {
			event := base
			change(&event)
			if filterSelects(t, filters, event) {
				t.Fatal("every clause must match")
			}
		})
	}
}

func TestCloudEventStatusFiltersNeedPresentFields(t *testing.T) {
	event := CloudEvent{Kind: "message.status", ChatID: "+15551234567", Status: "read", Data: json.RawMessage(`{}`)}
	status := map[string]any{"$eq": "message.status"}
	for _, test := range []struct {
		name    string
		filters map[string]any
		match   bool
	}{
		{"read receipt", map[string]any{"input.kind": status, "input.status": map[string]any{"$in": []string{"delivered", "read"}}, "input.chat_id": map[string]any{"$eq": "+15551234567"}}, true},
		{"all status types", map[string]any{"input.kind": status}, true},
		{"no kind filter", map[string]any{"input.chat_id": map[string]any{"$eq": "+15551234567"}}, true},
		{"wrong status", map[string]any{"input.kind": status, "input.status": map[string]any{"$eq": "failed"}}, false},
		{"wrong chat", map[string]any{"input.kind": status, "input.chat_id": map[string]any{"$eq": "+15559999999"}}, false},
		{"received only", map[string]any{"input.kind": map[string]any{"$eq": "message.received"}}, false},
		{"sender absent", map[string]any{"input.sender_id": map[string]any{"$eq": "+15551234567"}}, false},
		{"message type absent", map[string]any{"input.message_type": map[string]any{"$eq": "text"}}, false},
	} {
		t.Run(test.name, func(t *testing.T) {
			if got := filterSelects(t, test.filters, event); got != test.match {
				t.Fatalf("selected = %t, want %t", got, test.match)
			}
		})
	}
}

func TestCloudEventFiltersMatchRawAndFutureMessageTypes(t *testing.T) {
	filters := map[string]any{"input.message_type": map[string]any{"$in": []string{"interactive", "contacts", "poll", "future_type_2"}}}
	for _, kind := range []string{"interactive", "contacts", "poll", "future_type_2"} {
		if !filterSelects(t, filters, CloudEvent{Kind: "message.received", MessageType: kind, Data: json.RawMessage(`{}`)}) {
			t.Fatalf("raw Meta type %q did not match", kind)
		}
	}
	for _, kind := range []string{"text", "contact", "unknown"} {
		if filterSelects(t, filters, CloudEvent{Kind: "message.received", MessageType: kind, Data: json.RawMessage(`{}`)}) {
			t.Fatalf("archive preview type %q incorrectly matched a raw type filter", kind)
		}
	}
}

// The function description is the guide agents and ADE read: its examples
// must be filters iii accepts that select the events they describe.
func TestCloudEventGuideExamplesSelectTheirEvents(t *testing.T) {
	guide := cloudEventDescription("messaging")
	var blocks []string
	for rest := guide; ; {
		_, after, found := strings.Cut(rest, "```json\n")
		if !found {
			break
		}
		block, tail, found := strings.Cut(after, "\n```")
		if !found {
			t.Fatal("unterminated JSON example")
		}
		blocks = append(blocks, block)
		rest = tail
	}
	if len(blocks) != 5 {
		t.Fatalf("got %d JSON examples, want source, receipt filters, payload, template filters and group filters", len(blocks))
	}
	decode := func(block string) map[string]any {
		t.Helper()
		var value map[string]any
		if err := json.Unmarshal([]byte(block), &value); err != nil {
			t.Fatalf("invalid example %s: %v", block, err)
		}
		return value
	}
	source := decode(blocks[0])
	if source["function_id"] != "whatsapp::cloud::on-events" || source["namespace"] != "messaging" || source["deliver"] != "output" {
		t.Fatalf("guide source does not name this provider: %v", source)
	}
	if _, takesArgs := source["args"]; takesArgs {
		t.Fatal("whatsapp::cloud::on-events takes no args")
	}
	sourceFilters, _ := source["filters"].(map[string]any)
	receipts, templates, groups := decode(blocks[1]), decode(blocks[3]), decode(blocks[4])
	var message CloudEvent
	if err := json.Unmarshal([]byte(blocks[2]), &message); err != nil {
		t.Fatal(err)
	}
	raw, _ := json.Marshal(message)
	var value any
	_ = json.Unmarshal(raw, &value)
	if err := compileCloudEventSchema(t, cloudEventPayloadSchema).Validate(value); err != nil {
		t.Fatalf("documented payload is invalid: %v", err)
	}
	read := CloudEvent{Kind: "message.status", BusinessAccountID: "123", PhoneNumberID: "456", MessageID: "wamid.example", ChatID: "+15550000009", Status: "read", Data: json.RawMessage(`{}`)}
	template := CloudEvent{Kind: "webhook.received", BusinessAccountID: "123", Field: "message_template_status_update", Event: "APPROVED", Data: json.RawMessage(`{}`)}
	group := CloudEvent{Kind: "message.received", ChatID: "group:Y2FwaV9ncm91cDoxMjM=", SenderID: "+15550000009", MessageType: "text", GroupID: "Y2FwaV9ncm91cDoxMjM=", Data: json.RawMessage(`{}`)}
	for _, tc := range []struct {
		name    string
		filters map[string]any
		event   CloudEvent
		match   bool
	}{
		{"source selects the message", sourceFilters, message, true},
		{"source skips its receipt", sourceFilters, read, false},
		{"receipts select a read status", receipts, read, true},
		{"receipts skip the message", receipts, message, false},
		{"templates select an approval", templates, template, true},
		{"templates skip messages", templates, message, false},
		{"groups select a group message", groups, group, true},
		{"groups skip direct messages", groups, message, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if got := filterSelects(t, tc.filters, tc.event); got != tc.match {
				t.Fatalf("selected = %t, want %t", got, tc.match)
			}
		})
	}
}

func TestEmitCloudEventCallsTheSourceFunction(t *testing.T) {
	ctx := context.Background()
	event := CloudEvent{Kind: "message.received", ChatID: "+15551234567", SenderID: "+15551234567", MessageType: "text", MessageID: "wamid.example", Text: "Hello", TimestampMS: 123000, Data: json.RawMessage(`{"type":"text","text":{"body":"Hello"}}`), epoch: 42}
	calls := 0
	call := func(_ context.Context, request iii.CallRequest) (json.RawMessage, error) {
		calls++
		if request.FunctionID != "whatsapp::cloud::on-events" || request.Namespace != "" || request.Metadata != nil {
			t.Fatalf("event called the wrong function: %#v", request)
		}
		if request.Action == nil || request.Action.Type != "void" {
			t.Fatal("Cloud event emission must not wait for observers")
		}
		var received map[string]any
		if err := json.Unmarshal(request.Payload, &received); err != nil {
			t.Fatal(err)
		}
		if received["chat_id"] != event.ChatID || received["text"] != "Hello" || received["message_type"] != "text" || received["timestamp_ms"] != float64(123000) {
			t.Fatalf("event fields lost: %s", request.Payload)
		}
		if _, exists := received["epoch"]; exists {
			t.Fatal("private event epoch leaked")
		}
		data, _ := received["data"].(map[string]any)
		if data["type"] != "text" {
			t.Fatal("raw Meta message was not preserved")
		}
		return nil, nil
	}
	current := func(CloudEvent) bool { return true }
	emitCloudEvent(ctx, event, func() bool { return false }, current, call)
	if calls != 0 {
		t.Fatal("an unobserved event was emitted")
	}
	emitCloudEvent(ctx, event, func() bool { return true }, current, call)
	if calls != 1 {
		t.Fatalf("got %d calls, want one", calls)
	}
}

func TestEmitCloudEventStopsAfterAccountInvalidation(t *testing.T) {
	event := CloudEvent{Kind: "message.received"}
	observed := func() bool { return true }
	active := true
	calls := 0
	call := func(context.Context, iii.CallRequest) (json.RawMessage, error) {
		calls++
		active = false
		return nil, errors.New("test delivery failure")
	}
	emitCloudEvent(context.Background(), event, observed, func(CloudEvent) bool { return active }, call)
	if calls != 1 {
		t.Fatalf("current event was not emitted: %d calls", calls)
	}
	emitCloudEvent(context.Background(), event, observed, func(CloudEvent) bool { return active }, call)
	if calls != 1 {
		t.Fatal("stale account event was emitted")
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	emitCloudEvent(ctx, event, observed, func(CloudEvent) bool { return true }, call)
	if calls != 1 {
		t.Fatal("event emitted after shutdown")
	}
}
