package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"log"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestLegacyConfigRejectsUnprovenAbsence(t *testing.T) {
	for _, raw := range []string{`{}`, `null`, `[]`, `{"other":"private-sentinel"}`, `"private-sentinel"`} {
		t.Run(raw, func(t *testing.T) {
			clearCloudEnvironment(t)
			registered := make(chan map[string]json.RawMessage, 1)
			peer := legacyConfigPeer(t, raw, "", registered)
			ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
			defer cancel()
			_, err := ensureConfig(ctx, peer.client, DefaultConfig())
			if err == nil {
				t.Error("malformed configuration response must abort initialization")
			} else if strings.Contains(err.Error(), "private-sentinel") {
				t.Error("configuration response leaked into initialization error")
			}
			select {
			case <-registered:
				t.Error("malformed configuration response authorized registration")
			default:
			}
		})
	}
}

func TestLegacyConfigSeedsOnlyExplicitAbsence(t *testing.T) {
	for _, tc := range []struct {
		name, raw, code string
		seed            bool
	}{
		{"null value", `{"value":null}`, "", true},
		{"missing entry", "", "NOT_FOUND", true},
		{"false value", `{"value":false}`, "", false},
		{"zero value", `{"value":0}`, "", false},
		{"empty string", `{"value":""}`, "", false},
		{"saved template", `{"value":{"cloud":{"access_token":"${PRIVATE_SENTINEL}"}}}`, "", false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			clearCloudEnvironment(t)
			registered := make(chan map[string]json.RawMessage, 1)
			peer := legacyConfigPeer(t, tc.raw, tc.code, registered)
			ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
			defer cancel()
			if _, err := ensureConfig(ctx, peer.client, DefaultConfig()); err != nil {
				t.Fatal(err)
			}
			payload := <-registered
			if _, exists := payload["initial_value"]; exists != tc.seed {
				t.Fatalf("initial_value present=%t, want %t", exists, tc.seed)
			}
			var id string
			_ = json.Unmarshal(payload["id"], &id)
			if id != ConfigID() {
				t.Error("registration changed configuration entry")
			}
		})
	}
}

func TestConfigInitializationFallsBackOnlyOnExactCodes(t *testing.T) {
	for _, tc := range []struct{ ensure, get string }{
		{"NOT_FOUND", ""}, {"FUNCTION_NOT_FOUND", ""}, {"other", ""},
		{"function_not_found", "not_found"}, {"function_not_found", "function_not_found"}, {"function_not_found", "other"},
	} {
		t.Run(tc.ensure+"/"+tc.get, func(t *testing.T) {
			calls := make(chan string, 8)
			peer := newCallbackPeer(t, func(msg map[string]json.RawMessage) map[string]any {
				if len(msg["invocation_id"]) == 0 {
					return nil
				}
				var id string
				_ = json.Unmarshal(msg["function_id"], &id)
				calls <- id
				reply := callbackInvocationReply(msg)
				code := tc.ensure
				if id == "configuration::get" {
					code = tc.get
				}
				reply["error"] = map[string]any{"code": code, "message": "private-sentinel function_not_found NOT_FOUND"}
				return reply
			})
			ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
			defer cancel()
			if _, err := ensureConfig(ctx, peer.client, DefaultConfig()); err == nil || strings.Contains(err.Error(), "private-sentinel") {
				t.Fatalf("initialization must fail without exposing remote errors: %v", err)
			}
			close(calls)
			for id := range calls {
				if id == "configuration::register" || (tc.ensure != "function_not_found" && id != "configuration::ensure") {
					t.Fatalf("remote code authorized unexpected call %s", id)
				}
			}
		})
	}
}

func TestLegacyConfigWarningOncePerEntryAndSecretSafe(t *testing.T) {
	clearCloudEnvironment(t)
	entry := "legacy-warning-entry-" + filepath.Base(filepath.Dir(t.TempDir()))
	t.Setenv("III_CONFIG_NAME", entry)
	var output bytes.Buffer
	original := log.Writer()
	log.SetOutput(&output)
	t.Cleanup(func() { log.SetOutput(original) })
	registered := make(chan map[string]json.RawMessage, 2)
	peer := legacyConfigPeer(t, `{"value":{"cloud":{"access_token":"private-sentinel"}}}`, "", registered)
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	for range 2 {
		if _, err := ensureConfig(ctx, peer.client, DefaultConfig()); err != nil {
			t.Fatal(err)
		}
	}
	warning := "engine lacks configuration::ensure; using non-atomic legacy initialization; upgrade to >=0.24.1 for concurrent-write safety"
	if strings.Count(output.String(), warning) != 1 || !strings.Contains(output.String(), entry) {
		t.Fatalf("expected one identified compatibility warning, got %q", output.String())
	}
	if strings.Contains(output.String(), "private-sentinel") {
		t.Fatal("compatibility warning exposed configuration values")
	}
}

func TestConfigurationRegistrationRespectsCanceledManagerLock(t *testing.T) {
	config := DefaultConfig()
	config.DataDir = t.TempDir()
	manager, err := newTestManager(config)
	if err != nil {
		t.Fatal(err)
	}
	defer manager.Shutdown(context.Background())
	peer := newCallbackPeer(t, nil)
	manager.mu.Lock()
	defer manager.mu.Unlock()
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	done := make(chan error, 1)
	go func() { done <- registerConfiguration(ctx, peer.client, manager, true) }()
	select {
	case err := <-done:
		if !errors.Is(err, context.Canceled) {
			t.Fatalf("canceled registration returned %v", err)
		}
	case <-time.After(time.Second):
		t.Fatal("configuration registration kept waiting for the manager after cancellation")
	}
}

func legacyConfigPeer(t *testing.T, raw, getCode string, registered chan<- map[string]json.RawMessage) *callbackPeer {
	t.Helper()
	return newCallbackPeer(t, func(msg map[string]json.RawMessage) map[string]any {
		if len(msg["invocation_id"]) == 0 {
			return nil
		}
		var id string
		_ = json.Unmarshal(msg["function_id"], &id)
		reply := callbackInvocationReply(msg)
		switch id {
		case "configuration::ensure":
			reply["error"] = map[string]any{"code": "function_not_found", "message": "missing"}
		case "configuration::get":
			var data map[string]any
			_ = json.Unmarshal(msg["data"], &data)
			if data["raw"] == true {
				if getCode != "" {
					reply["error"] = map[string]any{"code": getCode, "message": "private-sentinel"}
				} else {
					reply["result"] = json.RawMessage(raw)
				}
			} else {
				reply["result"] = map[string]any{"value": DefaultConfig()}
			}
		case "configuration::register":
			var data map[string]json.RawMessage
			_ = json.Unmarshal(msg["data"], &data)
			registered <- data
			reply["result"] = map[string]any{}
		}
		return reply
	})
}
