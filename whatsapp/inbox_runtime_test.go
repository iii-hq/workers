package main

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/coder/websocket"
	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

func TestNativeInboxLockWaitHonorsRequestCancellation(t *testing.T) {
	m, s := testSessionManager(t)
	s.inbox.mu.Lock()
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	entered := make(chan struct{})
	registry := NewRegistry(m)
	handler := registry.handlers["whatsapp::messages::list"]
	original := handler.call
	handler.call = func(ctx context.Context, input map[string]any) (any, error) {
		close(entered)
		return original(ctx, input)
	}
	registry.handlers["whatsapp::messages::list"] = handler
	go func() {
		_, err := registry.Invoke(ctx, "whatsapp::messages::list", map[string]any{"session_id": "test", "chat_id": "15551234567@s.whatsapp.net"})
		done <- err
	}()
	<-entered
	cancel()
	select {
	case err := <-done:
		s.inbox.mu.Unlock()
		if !errors.Is(err, context.Canceled) {
			t.Fatalf("canceled inbox request returned %v", err)
		}
	case <-time.After(250 * time.Millisecond):
		s.inbox.mu.Unlock()
		<-done
		t.Fatal("canceled inbox request kept waiting for the inbox lock")
	}
}

func runtimeWebhookFixture(t *testing.T) (*Manager, Config, *Inbox, string, cloudReceiveRequest, <-chan struct{}) {
	t.Helper()
	config := cloudTestConfig()
	config.DataDir = t.TempDir()
	manager, err := newTestManager(config)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = manager.Shutdown(context.Background()) })
	inbox, err := manager.CloudInbox(config)
	if err != nil {
		t.Fatal(err)
	}
	raw, _ := json.Marshal(cloudTestPayload(cloudTestChange(cloudTestMessage("deadline"))))
	entered := make(chan struct{}, 16)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer conn.CloseNow()
		entered <- struct{}{}
		ctx, cancel := context.WithTimeout(r.Context(), time.Second)
		defer cancel()
		if err = conn.Write(ctx, websocket.MessageBinary, raw); err == nil {
			_ = conn.Close(websocket.StatusNormalClosure, "done")
		}
	}))
	t.Cleanup(server.Close)
	ref := &iii.StreamChannelRef{ChannelID: "test-channel", AccessKey: "test-key", Direction: iii.ChannelRead}
	req := cloudReceiveRequest{Headers: map[string]string{"X-Hub-Signature-256": cloudTestSignature(raw)}, RequestBody: ref}
	return manager, config, inbox, "ws" + strings.TrimPrefix(server.URL, "http"), req, entered
}

func TestCloudWebhookDeadlineDoesNotLeaveAnArchiveJob(t *testing.T) {
	manager, _, inbox, engineURL, req, entered := runtimeWebhookFixture(t)
	inbox.mu.Lock()
	ctx, cancel := context.WithTimeout(context.Background(), 100*time.Millisecond)
	defer cancel()
	done := make(chan cloudHTTPResponse, 1)
	go func() { done <- receiveCloudWebhook(ctx, manager, engineURL, req) }()
	<-entered
	response := <-done
	inbox.mu.Unlock()
	if response.StatusCode != 503 {
		t.Fatalf("blocked delivery returned %d", response.StatusCode)
	}
	select {
	case <-manager.CloudEventQueue:
		t.Fatal("delivery committed and published after its deadline")
	case <-time.After(200 * time.Millisecond):
	}
	page, err := inbox.ListMessages("cloud:+15551234567", "", 10)
	if err != nil || len(page.Messages) != 0 {
		t.Fatalf("timed out delivery left archive rows: %+v %v", page, err)
	}
}

func TestCloudWebhookRejectsDeliveryWhenAdmissionIsFull(t *testing.T) {
	manager, _, inbox, engineURL, req, entered := runtimeWebhookFixture(t)
	inbox.mu.Lock()
	ctx, cancel := context.WithCancel(context.Background())
	var requests sync.WaitGroup
	requests.Add(8)
	for range 8 {
		go func() { defer requests.Done(); receiveCloudWebhook(ctx, manager, engineURL, req) }()
	}
	for range 8 {
		<-entered
	}
	full := req
	full.RequestBody = nil
	response := receiveCloudWebhook(context.Background(), manager, engineURL, full)
	cancel()
	inbox.mu.Unlock()
	requests.Wait()
	if response.StatusCode != 503 {
		t.Fatalf("ninth concurrent delivery returned %d instead of an admission rejection", response.StatusCode)
	}
}

func TestSessionStopStateLockHonorsRequestDeadline(t *testing.T) {
	m, s := testSessionManager(t)
	s.mu.Lock()
	ctx, cancel := context.WithTimeout(context.Background(), 50*time.Millisecond)
	defer cancel()
	done := make(chan error, 1)
	go func() { _, err := m.Stop(ctx, "test", false); done <- err }()
	select {
	case err := <-done:
		s.mu.Unlock()
		if !errors.Is(err, context.DeadlineExceeded) {
			t.Fatalf("expired stop returned %v", err)
		}
	case <-time.After(250 * time.Millisecond):
		s.mu.Unlock()
		<-done
		t.Fatal("stop request continued waiting for the session state lock after its deadline")
	}
}
