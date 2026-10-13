package main

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
	"time"
)

func TestHTTPCallbacksCancelWaitingForConfiguration(t *testing.T) {
	for _, callback := range []string{"verify", "flow"} {
		for _, cancellation := range []string{"canceled", "deadline"} {
			t.Run(callback+"/"+cancellation, func(t *testing.T) {
				manager := &Manager{config: DefaultConfig()}
				manager.mu.Lock()
				defer manager.mu.Unlock()
				ctx, cancel := context.WithTimeout(context.Background(), 10*time.Millisecond)
				defer cancel()
				if cancellation == "canceled" {
					cancel()
				}
				done := make(chan cloudHTTPResponse, 1)
				go func() {
					response, err := invokeHTTPCallback(ctx, callback, manager, `{}`)
					if err != nil {
						response = cloudHTTPText(500, err.Error())
					}
					done <- response
				}()
				select {
				case response := <-done:
					if response.StatusCode != 503 || !strings.Contains(response.Body, "unavailable") {
						t.Fatalf("canceled configuration read returned %+v", response)
					}
				case <-time.After(time.Second):
					t.Fatal("HTTP callback kept waiting for configuration after its context ended")
				}
			})
		}
	}
}

func TestHTTPCallbacksPreserveMalformed400AfterCancellation(t *testing.T) {
	for _, tc := range []struct{ callback, raw string }{
		{"verify", `{"query_params":42}`},
		{"flow", `{"headers":42}`},
	} {
		t.Run(tc.callback, func(t *testing.T) {
			manager := &Manager{config: DefaultConfig()}
			manager.mu.Lock()
			defer manager.mu.Unlock()
			ctx, cancel := context.WithCancel(context.Background())
			cancel()
			response, err := invokeHTTPCallback(ctx, tc.callback, manager, tc.raw)
			if err != nil || response.StatusCode != 400 {
				t.Fatalf("malformed callback changed its HTTP response: %+v %v", response, err)
			}
		})
	}
}

func invokeHTTPCallback(ctx context.Context, callback string, manager *Manager, raw string) (cloudHTTPResponse, error) {
	if callback == "verify" {
		return cloudVerificationHandler(manager)(ctx, callbackRequest[cloudVerifyRequest]{Raw: json.RawMessage(raw)})
	}
	return cloudFlowEndpointHandler(nil, manager, "ws://127.0.0.1:9")(ctx, callbackRequest[cloudReceiveRequest]{Raw: json.RawMessage(raw)})
}
