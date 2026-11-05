package hub

import (
	"encoding/json"
	"sync"
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

func currentNodeEvent(t *testing.T, e *eventWire, client *iii.Client, binding *nodeBinding, name string) map[string]json.RawMessage {
	t.Helper()
	data, _ := json.Marshal(nodeEvent{Name: name, Event: "online", At: time.Now()})
	meta, _ := json.Marshal(nodeEventMetadata{ForObserver: "same", Generation: binding.generation})
	completed := make(chan error, 1)
	go func() {
		_, err := client.Call(e.ctx, iii.CallRequest{FunctionID: "mesh::on-node-changed", Payload: data, Metadata: meta, Timeout: time.Second})
		completed <- err
	}()
	select {
	case frame := <-e.frames:
		if string(frame["type"]) != `"invokefunction"` {
			t.Fatalf("unexpected frame: %s", frame)
		}
		var event nodeEvent
		if json.Unmarshal(frame["data"], &event) != nil || event.Name != name || event.Event != "online" {
			t.Fatalf("fresh payload: %s", frame)
		}
		e.ack(frame)
		if err := <-completed; err != nil {
			t.Fatal(err)
		}
		return frame
	case err := <-completed:
		t.Fatalf("fresh source completed without observer delivery: %v", err)
	case <-e.ctx.Done():
		t.Fatal("delivery did not complete")
	}
	return nil
}

// Same args with a changed target/filter/capture/on is an accepted SDK view
// refresh, not a new Mesh binding lifetime: neither lifecycle hook runs.
func TestNodeEventsCurrentGenerationAfterViewRefresh(t *testing.T) {
	for _, kind := range []string{"target", "filters", "capture", "on"} {
		t.Run(kind, func(t *testing.T) {
			h, _ := nodeCallHub(t)
			e := newEventWire(t, h)
			if !e.bind("same", `{"node":"laptop"}`) {
				t.Fatal("initial observer")
			}
			h.events.mu.Lock()
			binding := h.events.bindings["same"]
			h.events.mu.Unlock()
			info := iii.ObserverInfo{ID: "same", Args: json.RawMessage(`{"node":"laptop"}`), Filters: json.RawMessage(`{}`), Target: "audit::same", Capture: []iii.Capture{iii.CaptureInput, iii.CaptureOutput, iii.CaptureCaller}, On: iii.OnSuccess}
			switch kind {
			case "target":
				info.Target = "audit::updated"
			case "filters":
				info.Filters = json.RawMessage(`{"input.event":{"$eq":"online"}}`)
			case "capture":
				info.Capture = []iii.Capture{iii.CaptureInput}
			case "on":
				info.On = iii.OnComplete
			}
			if !e.announceObserver(info) {
				t.Fatal("view refresh rejected")
			}
			current := h.events.client.Observers("mesh::on-node-changed")
			if len(current) != 1 || current[0] == binding.observer {
				t.Fatal("SDK view did not change")
			}
			h.events.mu.Lock()
			unchanged := h.events.bindings["same"] == binding
			h.events.mu.Unlock()
			if !unchanged || binding.ctx.Err() != nil {
				t.Fatal("view refresh replaced the Mesh lifetime")
			}
			frame := currentNodeEvent(t, e, h.events.client, binding, "laptop")
			if kind == "target" && string(frame["function_id"]) != `"audit::updated"` {
				t.Fatalf("stale target: %s", frame)
			}
		})
	}
}

func TestNodeEventsRemovalAfterViewRefreshReleasesBinding(t *testing.T) {
	h, _ := nodeCallHub(t)
	e := newEventWire(t, h)
	if !e.bind("same", `{"node":"laptop"}`) {
		t.Fatal("initial observer")
	}
	h.events.mu.Lock()
	old := h.events.bindings["same"]
	h.events.mu.Unlock()
	if !e.announceObserver(iii.ObserverInfo{ID: "same", Args: json.RawMessage(`{"node":"laptop"}`), Target: "audit::updated", On: iii.OnSuccess}) {
		t.Fatal("view refresh")
	}
	e.revision++
	e.send(map[string]any{"type": "observerremoved", "function_id": "mesh::on-node-changed", "namespace": "hub-project", "revision": e.revision, "id": "same"})
	select {
	case <-old.ctx.Done():
	case <-time.After(time.Second):
		t.Fatal("removal of refreshed view retained the live binding")
	}
	h.events.mu.Lock()
	retained := h.events.bindings["same"] != nil
	h.events.mu.Unlock()
	if retained {
		t.Fatal("removed view leaked binding capacity")
	}
	if !e.bind("same", `{"node":"desktop"}`) {
		t.Fatal("reuse removed observer ID")
	}
	h.events.mu.Lock()
	fresh := h.events.bindings["same"]
	h.events.mu.Unlock()
	if fresh.generation == old.generation {
		t.Fatal("ID reuse retained old generation")
	}
	currentNodeEvent(t, e, h.events.client, fresh, "desktop")
}

func TestNodeEventsCapturedViewCannotDeliverAfterRefresh(t *testing.T) {
	h, _ := nodeCallHub(t)
	e := newEventWire(t, h)
	if !e.bind("same", `{"node":"laptop"}`) {
		t.Fatal("initial observer")
	}
	h.events.mu.Lock()
	binding := h.events.bindings["same"]
	h.events.mu.Unlock()
	entered, resume := make(chan struct{}), make(chan struct{})
	var pause, release sync.Once
	unblock := func() { release.Do(func() { close(resume) }) }
	defer unblock()
	h.events.client.OnObservationMatchFor("mesh::on-node-changed", iii.MatchFunc(func(obs *iii.Observer, call *iii.Call) bool {
		var meta nodeEventMetadata
		_ = json.Unmarshal(call.Metadata, &meta)
		pause.Do(func() { close(entered); <-resume })
		return meta.ForObserver == obs.ID
	}))
	eventVersion(t, h, "captured-old-view")
	select {
	case <-entered:
	case <-e.ctx.Done():
		t.Fatal("match did not start")
	}
	if !e.announceObserver(iii.ObserverInfo{ID: "same", Args: json.RawMessage(`{"node":"laptop"}`), Filters: json.RawMessage(`{"input.event":{"$eq":"online"}}`), Target: "audit::updated", On: iii.OnSuccess}) {
		t.Fatal("view refresh")
	}
	unblock()
	select {
	case frame := <-e.frames:
		t.Fatalf("stale captured view activated after refresh: %s", frame)
	case <-time.After(150 * time.Millisecond):
	}
	currentNodeEvent(t, e, h.events.client, binding, "laptop")
}
