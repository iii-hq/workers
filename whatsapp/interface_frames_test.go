package main

import (
	"context"
	"encoding/json"
	"reflect"
	"sort"
	"testing"
	"time"
)

// bootFrames runs the worker's boot registrations against a fake engine and
// returns the registerfunction frames by function id and the observe frames,
// once every expected function is registered and observeCount observers are sent.
func bootFrames(t *testing.T, observeCount int) (map[string]map[string]json.RawMessage, []map[string]json.RawMessage) {
	t.Helper()
	clearCloudEnvironment(t)
	t.Setenv("III_CONFIG_NAME", "wa-boot")
	t.Setenv("III_NAMESPACE", "")
	config, err := runtimeConfig(DefaultConfig())
	if err != nil {
		t.Fatal(err)
	}
	manager, err := newInterfaceManager(config)
	if err != nil {
		t.Fatal(err)
	}
	peer := newCallbackPeer(t, nil)
	peer.registerWorker(t)
	ctx, cancel := context.WithCancel(context.Background())
	t.Cleanup(cancel)
	if err := registerWorkerInterface(ctx, peer.client, manager, "ws://127.0.0.1:1", false); err != nil {
		t.Fatal(err)
	}
	want := make(map[string]bool)
	for _, entry := range NewRegistry(nil).Catalog() {
		want[entry.ID] = true
	}
	for _, id := range []string{"whatsapp::on-events", "whatsapp::cloud::on-events", "whatsapp::configuration-id", "whatsapp::config-status", "whatsapp::on-config-change", "whatsapp::ui-content", "whatsapp::cloud::webhook::verify", "whatsapp::cloud::webhook::receive", "whatsapp::cloud::flows::endpoint", "whatsapp::cloud::tunnel::changed"} {
		want[id] = true
	}
	registered := make(map[string]map[string]json.RawMessage)
	var observes []map[string]json.RawMessage
	deadline := time.After(5 * time.Second)
	for len(registered) < len(want) || len(observes) < observeCount {
		select {
		case frame := <-peer.frames:
			switch string(frame["type"]) {
			case `"registerfunction"`:
				var id string
				if err := json.Unmarshal(frame["id"], &id); err != nil {
					t.Fatal(err)
				}
				registered[id] = frame
			case `"observe"`:
				observes = append(observes, frame)
			}
		case <-deadline:
			t.Fatalf("boot sent %d of %d functions and %d of %d observers", len(registered), len(want), len(observes), observeCount)
		}
	}
	// Any further observe frame would be an observer the worker does not document.
	grace := time.After(200 * time.Millisecond)
	for done := false; !done; {
		select {
		case frame := <-peer.frames:
			if string(frame["type"]) == `"observe"` {
				observes = append(observes, frame)
			}
		case <-grace:
			done = true
		}
	}
	for id := range want {
		if registered[id] == nil {
			t.Errorf("boot did not register %s", id)
		}
	}
	return registered, observes
}

type observedSource struct {
	FunctionID string          `json:"function_id"`
	Args       map[string]any  `json:"args,omitempty"`
	Filters    map[string]any  `json:"filters,omitempty"`
	Deliver    string          `json:"deliver,omitempty"`
	Capture    json.RawMessage `json:"capture,omitempty"`
}

type observerFrame struct {
	Source observedSource `json:"source"`
	Target struct {
		FunctionID string `json:"function_id"`
	} `json:"target"`
}

func TestWorkerInterfaceObserveFrames(t *testing.T) {
	want := []observerFrame{
		{Source: observedSource{FunctionID: "configuration::on-updated", Filters: map[string]any{
			"input.id":         map[string]any{"$eq": "wa-boot"},
			"input.event_type": map[string]any{"$eq": "configuration:updated"},
		}, Deliver: "output"}},
		{Source: observedSource{FunctionID: "http::on-request", Args: map[string]any{"method": "GET", "path": "/webhooks/whatsapp-cloud/wa-boot", "public_webhook": true}}},
		{Source: observedSource{FunctionID: "http::on-request", Args: map[string]any{"method": "POST", "path": "/webhooks/whatsapp-cloud/wa-boot", "public_webhook": true}}},
		{Source: observedSource{FunctionID: "http::on-request", Args: map[string]any{"method": "POST", "path": "/webhooks/whatsapp-flows/wa-boot", "public_webhook": true}}},
		{Source: observedSource{FunctionID: "console::ui::asset", Args: map[string]any{"kind": "script", "path": "whatsapp/page.js"}}},
		{Source: observedSource{FunctionID: "console::ui::asset", Args: map[string]any{"kind": "style", "path": "whatsapp/styles.css"}}},
		{Source: observedSource{FunctionID: "quick-tunnel::on-changed", Filters: map[string]any{
			"input.tunnel_id": map[string]any{"$eq": "webhooks"},
		}, Deliver: "output"}},
	}
	targets := []string{
		"whatsapp::on-config-change",
		"whatsapp::cloud::webhook::verify",
		"whatsapp::cloud::webhook::receive",
		"whatsapp::cloud::flows::endpoint",
		"whatsapp::ui-content",
		"whatsapp::ui-content",
		"whatsapp::cloud::tunnel::changed",
	}
	for i := range want {
		want[i].Target.FunctionID = targets[i]
	}
	_, frames := bootFrames(t, len(want))
	got := make([]observerFrame, 0, len(frames))
	for _, frame := range frames {
		raw, err := json.Marshal(frame)
		if err != nil {
			t.Fatal(err)
		}
		var observer observerFrame
		if err := json.Unmarshal(raw, &observer); err != nil {
			t.Fatal(err)
		}
		if len(observer.Source.Capture) != 0 {
			t.Errorf("%s observer of %s sets capture %s; it uses the default", observer.Target.FunctionID, observer.Source.FunctionID, observer.Source.Capture)
		}
		observer.Source.Capture = nil
		got = append(got, observer)
	}
	key := func(o observerFrame) string {
		raw, _ := json.Marshal(o)
		return string(raw)
	}
	sort.Slice(got, func(i, j int) bool { return key(got[i]) < key(got[j]) })
	sort.Slice(want, func(i, j int) bool { return key(want[i]) < key(want[j]) })
	if !reflect.DeepEqual(got, want) {
		gotJSON, _ := json.MarshalIndent(got, "", "  ")
		wantJSON, _ := json.MarshalIndent(want, "", "  ")
		t.Fatalf("observe frames differ\ngot:  %s\nwant: %s", gotJSON, wantJSON)
	}
}
