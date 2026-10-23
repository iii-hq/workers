package main

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func TestInterfaceCaptureCallbacksCannotExecuteEnabledCloud(t *testing.T) {
	clearCloudEnvironment(t)
	config := DefaultConfig()
	config.DataDir = filepath.Join(t.TempDir(), "must-not-exist")
	config.Cloud.Enabled = true
	config.Cloud.BusinessAccountID = "123456789"
	config.Cloud.PhoneNumberID = "987654321"
	config.Cloud.AccessToken = "capture-access-private-sentinel"
	config.Cloud.AppSecret = "capture-app-private-sentinel"
	config.Cloud.VerifyToken = "capture-verify-private-sentinel"
	_, config.Cloud.FlowPrivateKey = flowTestKey(t)
	config.Cloud.FlowHandlerFunction = "capture::must-not-run"
	manager, err := newInterfaceManager(config)
	if err != nil {
		t.Fatal(err)
	}
	backendCalls := make(chan string, 16)
	peer := newCallbackPeer(t, func(msg map[string]json.RawMessage) map[string]any {
		if string(msg["type"]) != `"invokefunction"` {
			return nil
		}
		var id string
		_ = json.Unmarshal(msg["function_id"], &id)
		reply := callbackInvocationReply(msg)
		if id == "engine::workers::register" {
			reply["result"] = map[string]any{}
			return reply
		}
		backendCalls <- id
		reply["error"] = map[string]string{"code": "function_not_found", "message": "capture has no backends"}
		return reply
	})
	var channelCalls atomic.Int32
	channelServer := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		channelCalls.Add(1)
		http.Error(w, "capture must not read channels", http.StatusServiceUnavailable)
	}))
	defer channelServer.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if err := registerWorkerInterface(ctx, peer.client, manager, "ws"+strings.TrimPrefix(channelServer.URL, "http"), false); err != nil {
		t.Fatal(err)
	}
	verify, _ := json.Marshal(map[string]any{"query_params": map[string]string{"hub.mode": "subscribe", "hub.verify_token": config.Cloud.VerifyToken, "hub.challenge": "capture-challenge"}})
	receive := json.RawMessage(`{"headers":{"x-hub-signature-256":"sha256=` + strings.Repeat("0", 64) + `"},"request_body":{"channel_id":"capture-channel","access_key":"private-channel-key","direction":"read"}}`)
	for _, tc := range []struct {
		id, input, code, message string
		http                     bool
	}{
		{"whatsapp::cloud::webhook::verify", string(verify), "", "", true},
		{"whatsapp::cloud::webhook::receive", string(receive), "", "", true},
		{"whatsapp::cloud::flows::endpoint", string(receive), "", "", true},
		{"whatsapp::on-config-change", `{}`, "invalid_request", "Interface capture does not execute callbacks", false},
		{"whatsapp::cloud::tunnel::changed", `{"tunnel_id":"webhooks"}`, "invalid_request", "Interface capture does not execute callbacks", false},
		{"whatsapp::sessions::create", `{"session_id":"capture-session"}`, "invocation_failed", "worker is not initialized", false},
	} {
		t.Run(tc.id, func(t *testing.T) {
			raw, remote, err := peer.invoke(ctx, tc.id, json.RawMessage(tc.input))
			if err != nil {
				t.Fatal(err)
			}
			if tc.http {
				var response cloudHTTPResponse
				if remote != nil || json.Unmarshal(raw, &response) != nil || response.StatusCode != 503 || response.Body != "Interface capture does not execute callbacks" {
					t.Fatalf("capture HTTP callback executed or changed its safe response: %s %v", raw, remote)
				}
			} else if remote == nil || remote.Code != tc.code || remote.Message != tc.message {
				t.Fatalf("capture callback executed: %s %v", raw, remote)
			}
		})
	}
	raw, remote, err := peer.invoke(ctx, "whatsapp::config-status", json.RawMessage(`{}`))
	if err != nil || remote != nil {
		t.Fatalf("capture status: %v %v", err, remote)
	}
	var status ConfigStatus
	if json.Unmarshal(raw, &status) != nil || !status.Active.Cloud.Enabled || status.Active.Cloud.PhoneNumberID != "987654321" {
		t.Fatalf("capture status lost valid configured settings: %s", raw)
	}
	if strings.Contains(string(raw), "private-sentinel") || strings.Contains(string(raw), "PRIVATE KEY") || status.Active.Cloud.AccessToken != "" || status.Active.Cloud.AppSecret != "" || status.Active.Cloud.VerifyToken != "" || status.Active.Cloud.FlowPrivateKey != "" {
		t.Fatal("interface capture exposed configured credentials")
	}
	select {
	case id := <-backendCalls:
		t.Fatalf("interface capture invoked backend %s", id)
	default:
	}
	if channelCalls.Load() != 0 {
		t.Fatal("interface capture opened an HTTP body channel")
	}
	if _, err := os.Stat(config.DataDir); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("interface capture created its configured data directory: %v", err)
	}
	if manager.tunnel.done != nil || manager.tunnel.path != "" {
		t.Fatal("interface capture started tunnel reconciliation or inspected tunnel state")
	}
}
