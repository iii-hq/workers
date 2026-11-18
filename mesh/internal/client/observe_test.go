package client

import (
	"bytes"
	"context"
	"log"
	"os"
	"runtime/pprof"
	"strings"
	"testing"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

// A refused observer is never sent again and no event reaches it, so only the log can tell why the passes it starts never run.
func TestObserveLogsARefusal(t *testing.T) {
	var out lockedLog
	log.SetOutput(&out)
	t.Cleanup(func() { log.SetOutput(os.Stderr) })
	url, _ := meshHub(t, false) // an engine without observe
	c := iii.New(url, iii.WithOtel(iii.OtelConfig{Enabled: new(bool)}))
	defer c.Close()
	if err := c.Connect(t.Context()); err != nil {
		t.Fatal(err)
	}
	if err := Observe(t.Context(), c, iii.Source{FunctionID: "engine::on-functions-changed"}, "mesh::on-functions-change", iii.ObserverOptions{}, func() {}); err != nil {
		t.Fatal(err)
	}
	waitUntil(t, "the refusal in the log", func() bool {
		out.mu.Lock()
		defer out.mu.Unlock()
		return strings.Contains(out.b.String(), "mesh: observe engine::on-functions-changed: iii: observe rejected: UNSUPPORTED")
	})
}

// A closed client leaves its observers pending, so only ctx ends the wait for one to turn active.
func TestObserveWaitEndsWithItsContext(t *testing.T) {
	c := iii.New("ws://127.0.0.1:1", iii.WithOtel(iii.OtelConfig{Enabled: new(bool)})) // never connected: the observer stays pending
	defer c.Close()
	waiting := func() bool {
		var b bytes.Buffer
		_ = pprof.Lookup("goroutine").WriteTo(&b, 1)
		return strings.Contains(b.String(), "internal/client.keep.func")
	}
	ctx, cancel := context.WithCancel(t.Context())
	if err := Observe(ctx, c, iii.Source{FunctionID: "engine::on-functions-changed"}, "mesh::on-functions-change", iii.ObserverOptions{}, func() {}); err != nil {
		t.Fatal(err)
	}
	waitUntil(t, "the wait for the observer", waiting)
	cancel()
	waitUntil(t, "the wait to end with its context", func() bool { return !waiting() })
}
