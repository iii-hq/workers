package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"reflect"
	"strings"
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

func TestCloudEventFilterDefaultsToIncomingMessages(t *testing.T) {
	for _, raw := range []string{`{}`, `{"kinds":[]}`, `{"kinds":null}`} {
		filter, err := parseCloudEventFilter(json.RawMessage(raw))
		if err != nil {
			t.Fatal(err)
		}
		if !filter.matches(CloudEvent{Kind: "message.received"}) || filter.matches(CloudEvent{Kind: "message.status", Status: "read"}) {
			t.Fatalf("default subscription must select incoming messages only: %s", raw)
		}
	}
}

func TestCloudEventFilterNormalizesIdentitiesAndCombinesFilters(t *testing.T) {
	filter, err := parseCloudEventFilter(json.RawMessage(`{
		"kinds":["message.received","message.status"],
		"chat_ids":["+1 (555) 123-4567","BR.ENT.customer42"],
		"sender_ids":["15551234567","BR.ENT.customer42"],
		"message_types":["text","interactive"],
		"business_account_id":"123", "phone_number_id":"456"
	}`))
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(filter.ChatIDs, []string{"+15551234567", "BR.ENT.customer42"}) || !reflect.DeepEqual(filter.ChatIDs, filter.SenderIDs) {
		t.Fatalf("identities were not canonicalized: %#v", filter)
	}
	base := CloudEvent{Kind: "message.received", ChatID: "+15551234567", SenderID: "+15551234567", MessageType: "text", BusinessAccountID: "123", PhoneNumberID: "456"}
	if !filter.matches(base) {
		t.Fatal("matching incoming message rejected")
	}
	second := base
	second.ChatID, second.SenderID, second.MessageType = "BR.ENT.customer42", "BR.ENT.customer42", "interactive"
	if !filter.matches(second) {
		t.Fatal("OR values in each filter must match")
	}
	for name, change := range map[string]func(*CloudEvent){
		"kind":        func(e *CloudEvent) { e.Kind = "session.connected" },
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
			if filter.matches(event) {
				t.Fatal("all specified filters must match")
			}
		})
	}
}

func TestCloudEventStatusFiltersRequireMatchingKindAndPresentFields(t *testing.T) {
	event := CloudEvent{Kind: "message.status", ChatID: "+15551234567", Status: "read"}
	for _, test := range []struct {
		name, config string
		match        bool
	}{
		{"read receipt", `{"kinds":["message.status"],"statuses":["delivered","read"],"chat_ids":["15551234567"]}`, true},
		{"all status types", `{"kinds":["message.status"]}`, true},
		{"wrong status", `{"kinds":["message.status"],"statuses":["failed"]}`, false},
		{"wrong chat", `{"kinds":["message.status"],"chat_ids":["15559999999"]}`, false},
		{"default kind", `{"statuses":["read"]}`, false},
		{"sender absent", `{"kinds":["message.status"],"sender_ids":["15551234567"]}`, false},
		{"message type absent", `{"kinds":["message.status"],"message_types":["text"]}`, false},
	} {
		t.Run(test.name, func(t *testing.T) {
			filter, err := parseCloudEventFilter(json.RawMessage(test.config))
			if err != nil {
				t.Fatal(err)
			}
			if got := filter.matches(event); got != test.match {
				t.Fatalf("matches = %t, want %t", got, test.match)
			}
		})
	}
	filter, err := parseCloudEventFilter(json.RawMessage(`{"kinds":["message.received","message.status"],"statuses":["read"]}`))
	if err != nil || filter.matches(CloudEvent{Kind: "message.received"}) {
		t.Fatal("a status filter must exclude an incoming message without a status")
	}
}

func TestCloudEventFilterMatchesRawAndFutureMessageTypes(t *testing.T) {
	filter, err := parseCloudEventFilter(json.RawMessage(`{"message_types":["interactive","contacts","poll","future_type_2"]}`))
	if err != nil {
		t.Fatal(err)
	}
	for _, kind := range []string{"interactive", "contacts", "poll", "future_type_2"} {
		if !filter.matches(CloudEvent{Kind: "message.received", MessageType: kind}) {
			t.Fatalf("raw Meta type %q did not match", kind)
		}
	}
	for _, kind := range []string{"text", "contact", "unknown"} {
		if filter.matches(CloudEvent{Kind: "message.received", MessageType: kind}) {
			t.Fatalf("archive preview type %q incorrectly matched a raw type filter", kind)
		}
	}
	maxLength, _ := json.Marshal(map[string][]string{"message_types": {strings.Repeat("a", 64)}})
	if _, err := parseCloudEventFilter(maxLength); err != nil {
		t.Fatalf("64-character type must be accepted: %v", err)
	}
}

func TestCloudEventFilterRejectsInvalidAndOversizedConfiguration(t *testing.T) {
	for name, raw := range map[string]string{
		"empty": "", "null": "null", "array": "[]", "trailing": `{} {}`, "garbage": `{} trailing`,
		"unknown field": `{"sender_id":"15551234567"}`, "wrong type": `{"chat_ids":"15551234567"}`,
		"unknown kind": `{"kinds":["message"]}`, "blank kind": `{"kinds":[""]}`,
		"native group": `{"chat_ids":["123456@g.us"]}`, "native user": `{"sender_ids":["15551234567@s.whatsapp.net"]}`,
		"storage ID": `{"chat_ids":["cloud:+15551234567"]}`, "short phone": `{"chat_ids":["123"]}`,
		"invalid BSUID": `{"sender_ids":["BR.bad-id"]}`, "blank sender": `{"sender_ids":[""]}`,
		"blank type": `{"message_types":[""]}`, "wrong type case": `{"message_types":["Text"]}`,
		"malformed type": `{"message_types":["interactive.reply"]}`, "spaced type": `{"message_types":[" text"]}`,
		"numeric type": `{"message_types":["1text"]}`, "long type": `{"message_types":["` + strings.Repeat("a", 65) + `"]}`,
		"unknown status":  `{"statuses":["submitted"]}`,
		"invalid account": `{"business_account_id":"+123"}`, "invalid phone ID": `{"phone_number_id":"abc"}`,
		"long phone ID":  `{"phone_number_id":"123456789012345678901234567890123"}`,
		"oversized body": `{}` + strings.Repeat(" ", 64*1024),
	} {
		t.Run(name, func(t *testing.T) {
			if _, err := parseCloudEventFilter(json.RawMessage(raw)); err == nil {
				t.Fatalf("invalid filter accepted: %s", name)
			}
		})
	}
	for key, value := range map[string]string{"kinds": "message.received", "chat_ids": "15551234567", "sender_ids": "15551234567", "message_types": "text", "statuses": "read"} {
		t.Run(key+" limit", func(t *testing.T) {
			values := make([]string, 128)
			for i := range values {
				values[i] = value
			}
			valid, _ := json.Marshal(map[string]any{key: values})
			if _, err := parseCloudEventFilter(valid); err != nil {
				t.Fatalf("128 values must be accepted: %v", err)
			}
			invalid, _ := json.Marshal(map[string]any{key: append(values, value)})
			if _, err := parseCloudEventFilter(invalid); err == nil || !strings.Contains(err.Error(), key) {
				t.Fatal("129 values must identify the oversized filter")
			}
		})
	}
}

func TestCloudEventRouterPreservesRoutingAndOnlyDeliversMatchingEvents(t *testing.T) {
	ctx := context.Background()
	router := &CloudEventRouter{}
	for _, config := range []iii.TriggerConfig{
		{ID: "selected", FunctionID: "inbox::selected", Namespace: "customer-a", Metadata: json.RawMessage(`{"source":"subscription"}`), Config: json.RawMessage(`{"chat_ids":["15551234567"],"message_types":["text"]}`)},
		{ID: "other-chat", FunctionID: "inbox::other", Namespace: "customer-b", Config: json.RawMessage(`{"chat_ids":["15559999999"]}`)},
		{ID: "status", FunctionID: "inbox::status", Namespace: "customer-a", Config: json.RawMessage(`{"kinds":["message.status"]}`)},
	} {
		if err := router.RegisterTrigger(ctx, config); err != nil {
			t.Fatal(err)
		}
	}
	event := CloudEvent{Kind: "message.received", ChatID: "+15551234567", SenderID: "+15551234567", MessageType: "text", MessageID: "wamid.example", Text: "Hello", TimestampMS: 123000, Data: json.RawMessage(`{"type":"text","text":{"body":"Hello"}}`), epoch: 42}
	calls := 0
	trigger := func(callCtx context.Context, request iii.TriggerRequest) (json.RawMessage, error) {
		calls++
		if deadline, ok := callCtx.Deadline(); !ok || time.Until(deadline) > 5*time.Second {
			t.Fatal("dispatch must have a bounded deadline")
		}
		if request.FunctionID != "inbox::selected" || request.Namespace != "customer-a" || string(request.Metadata) != `{"source":"subscription"}` {
			t.Fatalf("routing or metadata lost: %#v", request)
		}
		if request.Action == nil || request.Action.Type != "void" {
			t.Fatal("Cloud event dispatch must not wait for the callback to complete")
		}
		var received map[string]any
		if err := json.Unmarshal(request.Data, &received); err != nil {
			t.Fatal(err)
		}
		if received["chat_id"] != event.ChatID || received["text"] != "Hello" || received["message_type"] != "text" || received["timestamp_ms"] != float64(123000) {
			t.Fatalf("event fields lost: %s", request.Data)
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
	router.deliver(ctx, event, func(CloudEvent) bool { return true }, trigger)
	if calls != 1 {
		t.Fatalf("got %d deliveries, want one matching subscription", calls)
	}
	if err := router.UnregisterTrigger(ctx, iii.TriggerConfig{ID: "selected"}); err != nil {
		t.Fatal(err)
	}
	router.deliver(ctx, event, func(CloudEvent) bool { return true }, trigger)
	if calls != 1 {
		t.Fatal("unregistered subscription received a subsequent message")
	}
}

func TestCloudEventRouterStopsDeliveryAfterAccountInvalidation(t *testing.T) {
	router := &CloudEventRouter{}
	for _, id := range []string{"one", "two"} {
		if err := router.RegisterTrigger(context.Background(), iii.TriggerConfig{ID: id, Config: json.RawMessage(`{}`)}); err != nil {
			t.Fatal(err)
		}
	}
	event := CloudEvent{Kind: "message.received"}
	active := true
	calls := 0
	trigger := func(context.Context, iii.TriggerRequest) (json.RawMessage, error) {
		calls++
		active = false
		return nil, errors.New("test delivery failure")
	}
	router.deliver(context.Background(), event, func(CloudEvent) bool { return active }, trigger)
	if calls != 1 {
		t.Fatalf("queued event crossed an account change: %d deliveries", calls)
	}
	router.deliver(context.Background(), event, func(CloudEvent) bool { return active }, trigger)
	if calls != 1 {
		t.Fatal("stale account event was dispatched")
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	router.deliver(ctx, event, func(CloudEvent) bool { return true }, trigger)
	if calls != 1 {
		t.Fatal("event dispatched after shutdown")
	}
}

func TestCloudEventRouterBoundsSubscriptionsAndAllowsReplacement(t *testing.T) {
	ctx := context.Background()
	router := &CloudEventRouter{}
	for i := range 1024 {
		if err := router.RegisterTrigger(ctx, iii.TriggerConfig{ID: fmt.Sprintf("subscription-%d", i), Config: json.RawMessage(`{}`)}); err != nil {
			t.Fatal(err)
		}
	}
	if err := router.RegisterTrigger(ctx, iii.TriggerConfig{ID: "overflow", Config: json.RawMessage(`{}`)}); err == nil {
		t.Fatal("subscription limit ignored")
	}
	replacement := iii.TriggerConfig{ID: "subscription-0", FunctionID: "updated::handler", Namespace: "updated", Config: json.RawMessage(`{"kinds":["message.status"]}`)}
	if err := router.RegisterTrigger(ctx, replacement); err != nil {
		t.Fatalf("existing subscription cannot be updated at capacity: %v", err)
	}
	if err := router.RegisterTrigger(ctx, iii.TriggerConfig{ID: replacement.ID, Config: json.RawMessage(`{"bad_filter":true}`)}); err == nil {
		t.Fatal("invalid replacement accepted")
	}
	if got := router.bindings[replacement.ID]; got.config.FunctionID != replacement.FunctionID || !got.filter.matches(CloudEvent{Kind: "message.status"}) {
		t.Fatal("invalid replacement changed an existing valid subscription")
	}
	if err := router.UnregisterTrigger(ctx, replacement); err != nil {
		t.Fatal(err)
	}
	if err := router.RegisterTrigger(ctx, iii.TriggerConfig{ID: "overflow", Config: json.RawMessage(`{}`)}); err != nil {
		t.Fatalf("unregistration did not release capacity: %v", err)
	}
}
