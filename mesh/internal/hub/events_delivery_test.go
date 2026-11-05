package hub

import (
	"encoding/json"
	"sync"
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/coordinator"
)

// The SDK captures observers before matching and deliberately detaches local
// delivery from caller cancellation. Invalidate that captured generation while
// matching is paused, before Mesh reaches its real observer delivery boundary.
func TestNodeEventsDeliveryRechecksGenerationAfterDelayedMatch(t *testing.T) {
	for _, transition := range []string{"replace", "remove", "disconnect", "resume", "current"} {
		t.Run(transition, func(t *testing.T) {
			h, _ := nodeCallHub(t)
			e := newEventWire(t, h)
			if !e.bind("same", `{"node":"laptop"}`) {
				t.Fatal("initial observer")
			}
			entered, resume := make(chan struct{}), make(chan struct{})
			var pause sync.Once
			var release sync.Once
			unblock := func() { release.Do(func() { close(resume) }) }
			defer unblock()
			h.events.client.OnObservationMatchFor("mesh::on-node-changed", iii.MatchFunc(func(obs *iii.Observer, call *iii.Call) bool {
				var meta struct {
					ForObserver string `json:"for_observer"`
				}
				_ = json.Unmarshal(call.Metadata, &meta)
				pause.Do(func() { close(entered); <-resume })
				return meta.ForObserver == obs.ID
			}))
			eventVersion(t, h, "captured-before-"+transition)
			select {
			case <-entered:
			case <-e.ctx.Done():
				t.Fatal("match never began")
			}
			h.events.mu.Lock()
			old := h.events.bindings["same"]
			h.events.mu.Unlock()
			switch transition {
			case "replace":
				if !e.bind("same", `{"node":"desktop"}`) {
					t.Fatal("replacement observer")
				}
			case "remove":
				e.revision++
				e.send(map[string]any{"type": "observerremoved", "function_id": "mesh::on-node-changed", "namespace": "hub-project", "revision": e.revision, "id": "same"})
			case "disconnect", "resume":
				if transition == "resume" {
					e.replayMu.Lock()
					e.replay = []iii.ObserverInfo{{ID: "same", Args: json.RawMessage(`{"node":"laptop"}`), Filters: json.RawMessage(`{}`), Target: "audit::same", Capture: []iii.Capture{iii.CaptureInput, iii.CaptureOutput, iii.CaptureCaller}, On: iii.OnSuccess}}
					e.replayMu.Unlock()
				}
				e.conn.CloseNow()
			}
			if transition != "current" {
				select {
				case <-old.ctx.Done():
				case <-time.After(time.Second):
					t.Fatal("generation was not canceled")
				}
			}
			if transition == "resume" {
				select {
				case e.conn = <-e.connections:
				case <-e.ctx.Done():
					t.Fatal("no resumed connection")
				}
				e.next("registerfunction")
				if f := e.next("observerack"); len(f["error"]) != 0 && string(f["error"]) != "null" {
					t.Fatalf("resume refused: %s", f)
				}
				h.events.mu.Lock()
				current := h.events.bindings["same"]
				h.events.mu.Unlock()
				if current.observer != old.observer {
					t.Fatal("fixture did not retain the SDK observer across resume")
				}
			}
			unblock()
			if transition == "current" {
				e.ack(e.delivery("laptop", "outdated"))
			} else {
				select {
				case frame := <-e.frames:
					t.Fatalf("%s delivered canceled laptop generation: %s", transition, frame)
				case <-time.After(150 * time.Millisecond):
				}
			}
			if transition == "disconnect" {
				select {
				case e.conn = <-e.connections:
				case <-e.ctx.Done():
					t.Fatal("no reconnect")
				}
				e.next("registerfunction")
			}
			if transition == "remove" || transition == "disconnect" {
				if !e.bind("same", `{"node":"desktop"}`) {
					t.Fatal("fresh observer")
				}
			}
			// The check must preserve legitimate current-generation deliveries.
			name := "desktop"
			if transition == "current" || transition == "resume" {
				name = "laptop"
			}
			h.events.publish(coordinator.NodeChange{Name: name, Event: "online", At: time.Now()})
			e.ack(e.delivery(name, "online"))
		})
	}
}
