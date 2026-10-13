package main

import (
	"bytes"
	"context"
	"errors"
	"io"
	"net/http"
	"testing"
	"time"
)

func TestEventPollCancelsWhileSessionStateIsLocked(t *testing.T) {
	manager, session := testSessionManager(t)
	registry := NewRegistry(manager)
	session.mu.Lock()
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Millisecond)
	defer cancel()
	done := make(chan error, 1)
	go func() {
		_, err := registry.Invoke(ctx, "whatsapp::events::poll", map[string]any{"session_id": "test"})
		done <- err
	}()
	select {
	case err := <-done:
		session.mu.Unlock()
		if !errors.Is(err, context.DeadlineExceeded) {
			t.Fatalf("expired poll returned %v", err)
		}
	case <-time.After(time.Second):
		session.mu.Unlock()
		<-done
		t.Fatal("event polling ignored cancellation while waiting for session state")
	}
}

func TestStopInvalidationFailurePreservesActiveSession(t *testing.T) {
	manager, session := testSessionManager(t)
	session.status, session.acceptEvents = "connected", true
	session.inbox.mu.Lock()
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Millisecond)
	defer cancel()
	_, err := manager.Stop(ctx, "test", false)
	session.inbox.mu.Unlock()
	if !errors.Is(err, context.DeadlineExceeded) {
		t.Fatalf("expected failed invalidation, got %v", err)
	}
	if session.status != "connected" || !session.acceptEvents {
		t.Fatalf("failed stop disabled active message callbacks: status=%s, accept_events=%v", session.status, session.acceptEvents)
	}
}

func TestNativeMediaUsesRequestLimitWithoutConfigurationRead(t *testing.T) {
	for _, method := range []string{http.MethodGet, http.MethodPost} {
		t.Run(method, func(t *testing.T) {
			reads := 0
			transport := nativeMediaTransport{
				maxBytes: func(context.Context) (int, error) { reads++; return 8, nil },
				base: nativeRoundTripFunc(func(*http.Request) (*http.Response, error) {
					return &http.Response{ContentLength: 0, Body: io.NopCloser(bytes.NewReader(nil))}, nil
				}),
			}
			ctx := context.WithValue(context.Background(), nativeMediaLimitKey{}, int64(8))
			request, _ := http.NewRequestWithContext(ctx, method, "https://mmg.whatsapp.net/mms/file", nil)
			response, err := transport.RoundTrip(request)
			if err != nil {
				t.Fatal(err)
			}
			response.Body.Close()
			if reads != 0 {
				t.Fatal("media request performed an unnecessary configuration lock wait")
			}
		})
	}
}

func TestTunnelStartCancelsWhileConfigurationIsLocked(t *testing.T) {
	manager, _ := testSessionManager(t)
	tunnel, _ := newTunnelFixture(t)
	tunnel.config = manager.ConfigContext
	manager.mu.Lock()
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Millisecond)
	defer cancel()
	done := make(chan error, 1)
	go func() { _, err := tunnel.Start(ctx); done <- err }()
	select {
	case err := <-done:
		manager.mu.Unlock()
		if !errors.Is(err, context.DeadlineExceeded) {
			t.Fatalf("expired tunnel start returned %v", err)
		}
	case <-time.After(time.Second):
		manager.mu.Unlock()
		<-done
		t.Fatal("tunnel startup ignored cancellation while waiting for configuration")
	}
}
