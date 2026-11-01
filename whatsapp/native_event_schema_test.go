package main

import (
	"context"
	"encoding/json"
	"reflect"
	"testing"
	"time"

	"github.com/iii-hq/iii/sdk/packages/go/iii/filter"
	validate "github.com/santhosh-tekuri/jsonschema/v6"
)

// filterSelects evaluates observer filters on an event the way iii does for an
// event source, whose input and output are both the event.
func filterSelects(t *testing.T, filters map[string]any, event any) bool {
	t.Helper()
	compiled, err := filter.Compile(filters)
	if err != nil {
		t.Fatalf("invalid filters %v: %v", filters, err)
	}
	raw, err := json.Marshal(event)
	if err != nil {
		t.Fatal(err)
	}
	var value any
	if err := json.Unmarshal(raw, &value); err != nil {
		t.Fatal(err)
	}
	return compiled.Matches(filter.Map{"input": value, "output": value})
}

func TestNativeEventFunctionWireSchemas(t *testing.T) {
	peer := newCallbackPeer(t, nil)
	peer.registerWorker(t)
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
			if string(received["type"]) == `"registerfunction"` && string(received["id"]) == `"whatsapp::on-events"` {
				frame = received
			}
		case <-ctx.Done():
			t.Fatal("native event function registration was not sent")
		}
	}
	if string(frame["private"]) != "true" {
		t.Error("whatsapp::on-events must be private: only this worker calls it")
	}
	if len(frame["args_format"]) != 0 {
		t.Error("whatsapp::on-events takes no args; observers filter on its input")
	}
	formats := make(map[string]any)
	for _, field := range []string{"request_format", "response_format"} {
		var document map[string]any
		if err := json.Unmarshal(frame[field], &document); err != nil || len(document) == 0 {
			t.Fatalf("native event function sends no typed %s", field)
		}
		formats[field] = document
	}
	if !reflect.DeepEqual(formats["request_format"], formats["response_format"]) {
		t.Fatal("the event source must return the event it is called with")
	}
	compiler := validate.NewCompiler()
	uri := "https://whatsapp.invalid/native-event-test.json"
	if err := compiler.AddResource(uri, formats["response_format"]); err != nil {
		t.Fatal(err)
	}
	schema, err := compiler.Compile(uri)
	if err != nil {
		t.Fatalf("invalid native event format: %v", err)
	}
	t.Run("event preserves generic data", func(t *testing.T) {
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
	t.Run("filters select session and kind", func(t *testing.T) {
		filters := map[string]any{
			"input.session_id": map[string]any{"$eq": "support"},
			"input.kind":       map[string]any{"$in": []string{"session.connected", "message.received"}},
		}
		for _, tc := range []struct {
			event Event
			match bool
		}{
			{Event{SessionID: "support", Kind: "message.received", Data: map[string]any{}}, true},
			{Event{SessionID: "support", Kind: "session.connected", Data: map[string]any{}}, true},
			{Event{SessionID: "support", Kind: "message.receipt", Data: map[string]any{}}, false},
			{Event{SessionID: "sales", Kind: "message.received", Data: map[string]any{}}, false},
		} {
			if got := filterSelects(t, filters, tc.event); got != tc.match {
				t.Errorf("%s/%s: selected = %t, want %t", tc.event.SessionID, tc.event.Kind, got, tc.match)
			}
		}
	})
}
