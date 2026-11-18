package coordinator

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/json"
	"net/http/httptest"
	"testing"
	"time"

	"tailscale.com/tailcfg"
	"tailscale.com/tstest"
	"tailscale.com/types/key"
)

func expectNodeChange(t *testing.T, events <-chan NodeChange, name, event string) {
	t.Helper()
	select {
	case got := <-events:
		if got.Name != name || got.Event != event || got.At.IsZero() {
			t.Fatalf("change %+v, want %s %s", got, name, event)
		}
	case <-time.After(time.Second):
		t.Fatalf("missing %s %s", name, event)
	}
}
func TestNodeObserverCommittedTransitions(t *testing.T) {
	c := newCoordinator(t, t.TempDir())
	events := make(chan NodeChange, 32)
	c.SetNodeObserver(func(e NodeChange) { events <- e })
	registerNode(t, c, "hub")
	machine, k := registerNode(t, c, "laptop")
	expectNodeChange(t, events, "laptop", "joined")
	req := tailcfg.RegisterRequest{NodeKey: k}
	if !c.register(machine, "a", req).MachineAuthorized {
		t.Fatal("reconnect")
	}
	metadata := RuntimeMetadata{OS: "linux", Arch: "amd64", Version: "old"}
	if err := c.SetRuntimeMetadata("laptop", metadata); err != nil {
		t.Fatal(err)
	}
	expectNodeChange(t, events, "laptop", "version")
	if err := c.SetRuntimeMetadata("laptop", metadata); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	raw, _ := json.Marshal(tailcfg.MapRequest{NodeKey: k, Stream: true})
	done := make(chan struct{})
	go func() {
		defer close(done)
		c.serveMap(httptest.NewRecorder(), httptest.NewRequest("POST", "/machine/map", bytes.NewReader(raw)).WithContext(ctx), machine)
	}()
	expectNodeChange(t, events, "laptop", "online")
	cancel()
	<-done
	expectNodeChange(t, events, "laptop", "offline")
	gone := make(chan string, 1)
	c.SetGone(func(s string) { gone <- s })
	if err := c.Remove("laptop"); err != nil {
		t.Fatal(err)
	}
	expectNodeChange(t, events, "laptop", "removed")
	select {
	case <-gone:
	case <-time.After(time.Second):
		t.Fatal("lost removal hook")
	}
	if len(events) != 0 {
		t.Fatal("duplicate or hub events", len(events))
	}
}
func TestNodeObserverRestoredBaselineAndEphemeralExpiry(t *testing.T) {
	clock := asyncClock{tstest.NewClock(tstest.ClockOpts{})}
	dir := t.TempDir()
	c, err := newWithClock(dir, func(string, string) (string, error) { return "", nil }, clock)
	if err != nil {
		t.Fatal(err)
	}
	req := tailcfg.RegisterRequest{NodeKey: key.NewNode().Public(), Ephemeral: true, Auth: &tailcfg.RegisterResponseAuth{AuthKey: rand.Text()}, Hostinfo: &tailcfg.Hostinfo{Hostname: "runner"}}
	if !c.register(key.NewMachine().Public(), "a", req).MachineAuthorized {
		t.Fatal("register")
	}
	c.Close()
	c, err = newWithClock(dir, func(string, string) (string, error) { return "", nil }, clock)
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	events := make(chan NodeChange, 8)
	c.SetNodeObserver(func(e NodeChange) { events <- e })
	if len(events) != 0 {
		t.Fatal("restored join replayed")
	}
	clock.Advance(3 * time.Minute)
	expectNodeChange(t, events, "runner", "removed")
	if len(c.Nodes()) != 0 {
		t.Fatal("expiry not committed")
	}
}

func TestNodeObserverFailedWritesDoNotPublish(t *testing.T) {
	c := newCoordinator(t, t.TempDir())
	machine, k := registerNode(t, c, "laptop")
	events := make(chan NodeChange, 8)
	c.SetNodeObserver(func(e NodeChange) { events <- e })
	c.file = t.TempDir()
	if err := c.SetRuntimeMetadata("laptop", RuntimeMetadata{OS: "linux", Arch: "amd64", Version: "old"}); err == nil {
		t.Fatal("metadata save succeeded")
	}
	if err := c.Remove("laptop"); err == nil {
		t.Fatal("removal save succeeded")
	}
	req := tailcfg.RegisterRequest{NodeKey: key.NewNode().Public(), Auth: &tailcfg.RegisterResponseAuth{AuthKey: rand.Text()}, Hostinfo: &tailcfg.Hostinfo{Hostname: "desktop"}}
	if c.register(key.NewMachine().Public(), "a", req).MachineAuthorized {
		t.Fatal("join save succeeded")
	}
	raw, _ := json.Marshal(tailcfg.MapRequest{NodeKey: k, Stream: true})
	response := httptest.NewRecorder()
	c.serveMap(response, httptest.NewRequest("POST", "/machine/map", bytes.NewReader(raw)), machine)
	if response.Code != 503 {
		t.Fatalf("failed map status %d", response.Code)
	}
	if len(events) != 0 {
		t.Fatal("failed transaction published", len(events))
	}
}

func TestNodeObserverOverlappingStreamsAndLogout(t *testing.T) {
	c := newCoordinator(t, t.TempDir())
	machine, k := registerNode(t, c, "laptop")
	events := make(chan NodeChange, 16)
	c.SetNodeObserver(func(e NodeChange) { events <- e })
	start := func() (context.CancelFunc, <-chan struct{}) {
		ctx, cancel := context.WithCancel(t.Context())
		t.Cleanup(cancel)
		raw, _ := json.Marshal(tailcfg.MapRequest{NodeKey: k, Stream: true})
		recorder := &pollRecorder{ResponseRecorder: httptest.NewRecorder(), flushed: make(chan struct{}, 16)}
		done := make(chan struct{})
		go func() {
			defer close(done)
			c.serveMap(recorder, httptest.NewRequest("POST", "/machine/map", bytes.NewReader(raw)).WithContext(ctx), machine)
		}()
		select {
		case <-recorder.flushed:
		case <-time.After(time.Second):
			t.Fatal("stream missing")
		}
		return cancel, done
	}
	cancelFirst, firstDone := start()
	expectNodeChange(t, events, "laptop", "online")
	cancelSecond, secondDone := start()
	cancelFirst()
	<-firstDone
	if len(events) != 0 {
		t.Fatal("overlapping stream fabricated connectivity change")
	}
	cancelSecond()
	<-secondDone
	expectNodeChange(t, events, "laptop", "offline")
	_, thirdDone := start()
	expectNodeChange(t, events, "laptop", "online")
	response := c.register(machine, "a", tailcfg.RegisterRequest{NodeKey: k, Expiry: time.Now().Add(-time.Minute)})
	if !response.NodeKeyExpired {
		t.Fatalf("logout %+v", response)
	}
	expectNodeChange(t, events, "laptop", "removed")
	select {
	case <-thirdDone:
	case <-time.After(time.Second):
		t.Fatal("logout left map open")
	}
	if len(events) != 0 {
		t.Fatal("removed node emitted offline after removal")
	}
}
