package main

import (
	"context"
	"encoding/json"
	"errors"
	"testing"
	"time"
)

func TestPublicOperationsRemainVisibleInTraces(t *testing.T) {
	peer := newCallbackPeer(t, nil)
	registry := NewRegistry(nil)
	if err := registry.Install(peer.client); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	remaining := len(registry.Catalog())
	for remaining > 0 {
		select {
		case frame := <-peer.frames:
			if string(frame["type"]) != `"registerfunction"` {
				continue
			}
			var metadata struct {
				Hidden bool `json:"trace_hidden"`
			}
			if err := json.Unmarshal(frame["metadata"], &metadata); err != nil {
				t.Fatal(err)
			}
			if metadata.Hidden {
				t.Errorf("public operation %s is hidden from traces", frame["id"])
			}
			remaining--
		case <-ctx.Done():
			t.Fatalf("missing %d public registrations: %v", remaining, ctx.Err())
		}
	}
}

func TestRegistryCancelsWhileWaitingForSessionTransition(t *testing.T) {
	manager, session := testSessionManager(t)
	registry := NewRegistry(manager)
	session.operations.Lock()
	defer session.operations.Unlock()
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() {
		_, err := registry.Invoke(ctx, "whatsapp::messages::list", map[string]any{"session_id": "test", "chat_id": "15551234567@s.whatsapp.net"})
		done <- err
	}()
	cancel()
	select {
	case err := <-done:
		if !errors.Is(err, context.Canceled) {
			t.Fatalf("canceled request returned %v", err)
		}
	case <-time.After(time.Second):
		t.Fatal("request continued waiting for a session transition after cancellation")
	}
}
