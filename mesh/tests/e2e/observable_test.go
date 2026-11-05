//go:build e2e

package e2e

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

func TestE2EHubObservableUIAndNodeEvents(t *testing.T) {
	for _, ns := range []string{"default", "project"} {
		t.Run(ns, func(t *testing.T) {
			f := newFixture(t, ns)
			// Host the actual ADE observable contract on the isolated engine. Mesh's
			// registrations must reach it; content is fetched through its real observers.
			assets := make(chan *iii.Observer, 4)
			f.admin.OnObserverAddedFor("ade::ui::asset", func(_ context.Context, obs *iii.Observer) error { assets <- obs; return nil })
			if _, err := iii.RegisterFunctionTyped(f.admin, "ade::ui::asset", func(context.Context, json.RawMessage) (bool, error) { return true, nil }, iii.RegisterFunctionOptions{ArgsFormat: json.RawMessage(`{"type":"object","properties":{"path":{"type":"string"},"kind":{"type":"string"}},"additionalProperties":true}`)}); err != nil {
				t.Fatal(err)
			}
			seen := map[string]bool{}
			for len(seen) < 2 {
				select {
				case obs := <-assets:
					var args struct{ Path, Kind string }
					if err := obs.DecodeArgs(&args); err != nil {
						t.Fatal(err)
					}
					if !strings.HasPrefix(args.Path, "mesh/") {
						t.Fatalf("foreign asset: %+v", args)
					}
					raw, err := f.admin.Call(f.ctx, iii.CallRequest{Observer: obs, Payload: json.RawMessage(mustJSON(map[string]any{"path": args.Path})), Timeout: 5 * time.Second})
					if err != nil {
						t.Fatal(err)
					}
					var content struct {
						Content     string `json:"content"`
						ContentType string `json:"content_type"`
					}
					if json.Unmarshal(raw, &content) != nil || len(content.Content) < 100 {
						t.Fatalf("asset content missing: %s", raw)
					}
					if (args.Kind == "script" && !strings.Contains(content.ContentType, "javascript")) || (args.Kind == "style" && !strings.Contains(content.ContentType, "css")) {
						t.Fatalf("asset MIME mismatch: %+v", args)
					}
					seen[args.Path] = true
				case <-time.After(5 * time.Second):
					t.Fatal("Mesh ADE asset observers did not reach current engine")
				}
			}
			if !seen["mesh/page.js"] || !seen["mesh/styles.css"] {
				t.Fatal(seen)
			}
			// Inventory created without observers must remain a baseline, not a replay.
			f.enroll("baseline")
			events := make(chan json.RawMessage, 32)
			if _, err := iii.RegisterFunctionTyped(f.admin, "audit::node-change", func(_ context.Context, p json.RawMessage) (bool, error) { events <- p; return true, nil }); err != nil {
				t.Fatal(err)
			}
			history := observe(t, f.admin, iii.Source{FunctionID: "mesh::on-node-changed", Namespace: ns, Deliver: iii.DeliverInput}, "audit::node-change", "history", true)
			select {
			case raw := <-events:
				t.Fatalf("replayed historical node: %s", raw)
			case <-time.After(150 * time.Millisecond):
			}
			if err := history.Dispose(f.ctx); err != nil {
				t.Fatal(err)
			}
			h := observe(t, f.admin, iii.Source{FunctionID: "mesh::on-node-changed", Namespace: ns, Args: iii.Args{"node": "watched"}, Deliver: iii.DeliverInput}, "audit::node-change", "watched", true)
			defer h.Dispose(f.ctx)
			f.enroll("other")
			select {
			case raw := <-events:
				t.Fatalf("node filter leaked: %s", raw)
			case <-time.After(150 * time.Millisecond):
			}
			f.enroll("watched")
			joined, online := false, false
			eventually(t, 5*time.Second, "node joined/online deliveries", func() error {
				deadline := time.After(3 * time.Second)
				for !joined || !online {
					select {
					case raw := <-events:
						var e struct{ Name, Event string }
						if json.Unmarshal(raw, &e) != nil || e.Name != "watched" {
							t.Fatalf("bad node event: %s", raw)
						}
						joined = joined || e.Event == "joined"
						online = online || e.Event == "online"
					case <-deadline:
						return fmt.Errorf("joined=%v online=%v", joined, online)
					}
				}
				return nil
			})
			mustCall(t, f.admin, ns, "mesh::nodes::remove", map[string]any{"name": "watched"})
			select {
			case raw := <-events:
				if !strings.Contains(string(raw), `"event":"removed"`) {
					t.Fatalf("removed event: %s", raw)
				}
			case <-time.After(5 * time.Second):
				t.Fatal("missing removed event")
			}
			t.Log("current ADE observers serve rebuilt JS/CSS; node joined/online/removed delivery, node filter and no historical replay verified")
		})
	}
}
