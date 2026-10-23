package main

import (
	"context"
	"encoding/json"
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	validate "github.com/santhosh-tekuri/jsonschema/v6"
)

func TestNativeEventTriggerWireSchemas(t *testing.T) {
	peer := newCallbackPeer(t, nil)
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	manager := &Manager{EventQueue: make(chan Event)}
	if err := registerEvents(ctx, peer.client, manager); err != nil {
		t.Fatal(err)
	}
	var frame map[string]json.RawMessage
	for frame == nil {
		select {
		case received := <-peer.frames:
			if string(received["type"]) == `"registertriggertype"` && string(received["id"]) == `"whatsapp::events"` {
				frame = received
			}
		case <-ctx.Done():
			t.Fatal("native event trigger registration was not sent")
		}
	}
	schemas := make(map[string]*validate.Schema)
	for _, field := range []string{"trigger_request_format", "call_request_format"} {
		var document map[string]any
		if err := json.Unmarshal(frame[field], &document); err != nil || len(document) == 0 {
			t.Errorf("native event trigger sends no typed %s", field)
			continue
		}
		compiler := validate.NewCompiler()
		uri := "https://whatsapp.invalid/native-event-test.json"
		if err := compiler.AddResource(uri, document); err != nil {
			t.Fatal(err)
		}
		schema, err := compiler.Compile(uri)
		if err != nil {
			t.Fatalf("invalid native event %s: %v", field, err)
		}
		schemas[field] = schema
	}
	if len(schemas) != 2 {
		return
	}
	t.Run("filter matches router", func(t *testing.T) {
		schema := schemas["trigger_request_format"]
		for _, tc := range []struct {
			raw   string
			valid bool
		}{
			{`{}`, true}, {`null`, true},
			{`{"session_id":null,"kinds":null}`, true},
			{`{"session_id":"","kinds":[]}`, true},
			{`{"session_id":"native-session_1","kinds":["message.received","Future.Event_2"]}`, true},
			{`[]`, false}, {`42`, false},
			{`{"unknown":true}`, false},
			{`{"session_id":"../outside"}`, false},
			{`{"session_id":42}`, false},
			{`{"kinds":"message.received"}`, false},
			{`{"kinds":[null]}`, false},
			{`{"kinds":[""]}`, false},
			{`{"kinds":["bad kind"]}`, false},
		} {
			t.Run(tc.raw, func(t *testing.T) {
				var value any
				if err := json.Unmarshal([]byte(tc.raw), &value); err != nil {
					t.Fatal(err)
				}
				router := &EventRouter{}
				parseErr := router.RegisterTrigger(ctx, iii.TriggerConfig{ID: "fixture", Config: json.RawMessage(tc.raw)})
				schemaErr := schema.Validate(value)
				if (parseErr == nil) != tc.valid || (schemaErr == nil) != tc.valid {
					t.Fatalf("valid=%t; router=%v; registered schema=%v", tc.valid, parseErr, schemaErr)
				}
			})
		}
		for _, count := range []int{128, 129} {
			kinds := make([]string, count)
			for i := range kinds {
				kinds[i] = "message.received"
			}
			raw, err := json.Marshal(map[string]any{"kinds": kinds})
			if err != nil {
				t.Fatal(err)
			}
			var value any
			_ = json.Unmarshal(raw, &value)
			router := &EventRouter{}
			parseErr := router.RegisterTrigger(ctx, iii.TriggerConfig{ID: "fixture", Config: raw})
			schemaErr := schema.Validate(value)
			if (parseErr == nil) != (count == 128) || (schemaErr == nil) != (count == 128) {
				t.Fatalf("kind count=%d; router=%v; registered schema=%v", count, parseErr, schemaErr)
			}
		}
	})
	t.Run("event preserves generic data", func(t *testing.T) {
		schema := schemas["call_request_format"]
		for _, data := range []any{nil, true, "value", []any{1, "two"}, map[string]any{"future": map[string]any{"content_omitted": true}}} {
			raw, err := json.Marshal(Event{SessionID: "native", Sequence: 17, Kind: "Future.Event_2", TimestampMS: 1790870400000, Data: data})
			if err != nil {
				t.Fatal(err)
			}
			var value map[string]any
			if err := json.Unmarshal(raw, &value); err != nil {
				t.Fatal(err)
			}
			value["_caller_worker_id"] = "transport-metadata"
			if err := schema.Validate(value); err != nil {
				t.Fatalf("native emitted event rejected: %v", err)
			}
			for _, required := range []string{"session_id", "sequence", "kind", "timestamp_ms", "data"} {
				original := value[required]
				delete(value, required)
				if err := schema.Validate(value); err == nil {
					t.Errorf("event envelope accepts missing %s", required)
				}
				value[required] = original
			}
			value["sequence"] = -1
			if err := schema.Validate(value); err == nil {
				t.Fatal("event envelope accepts a negative uint64 sequence")
			}
		}
	})
}
