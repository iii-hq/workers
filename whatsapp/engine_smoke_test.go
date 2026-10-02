package main

import (
	"context"
	"crypto/hmac"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

const smokeNamespace = "whatsapp-go-smoke"

func smokeProcess(t *testing.T, dir, name, binary string, args []string, environment ...string) {
	t.Helper()
	log, err := os.Create(filepath.Join(dir, name+".log"))
	if err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command(binary, args...)
	cmd.Dir = dir
	cmd.Env = append(os.Environ(), environment...)
	cmd.Stdout = log
	cmd.Stderr = log
	if err = cmd.Start(); err != nil {
		_ = log.Close()
		t.Fatal(err)
	}
	t.Cleanup(func() {
		_ = cmd.Process.Kill()
		_ = cmd.Wait()
		_ = log.Close()
		if t.Failed() {
			content, _ := os.ReadFile(filepath.Join(dir, name+".log"))
			t.Logf("%s log:\n%s", name, content)
		}
	})
}
func smokeTrigger(engine, engineURL, namespace, function string, payload any) (map[string]any, error) {
	value, err := smokeValue(engine, engineURL, namespace, function, payload)
	if err != nil {
		return nil, err
	}
	object, ok := value.(map[string]any)
	if !ok {
		return nil, fmt.Errorf("%s returned a non-object: %v", function, value)
	}
	return object, nil
}
func smokeValue(engine, engineURL, namespace, function string, payload any) (any, error) {
	encoded, err := json.Marshal(payload)
	if err != nil {
		return nil, err
	}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, engine, "trigger", "--engine", engineURL, "--namespace", namespace, function, "--json", string(encoded))
	var stderr strings.Builder
	cmd.Stderr = &stderr
	raw, err := cmd.Output()
	if err != nil {
		return nil, fmt.Errorf("%s: %v: %s", function, err, stderr.String())
	}
	if len(raw) == 0 {
		return nil, nil
	}
	var result any
	err = json.Unmarshal(raw, &result)
	return result, err
}
func smokeWait(t *testing.T, label string, probe func() bool) {
	t.Helper()
	deadline := time.Now().Add(45 * time.Second)
	for !probe() {
		if time.Now().After(deadline) {
			t.Fatalf("%s did not become ready", label)
		}
		time.Sleep(100 * time.Millisecond)
	}
}

// Run explicitly with III_BINARY and WHATSAPP_BINARY pointing to existing local
// binaries. HTTP_BINARY additionally enables the restricted-listener webhook test.
// Every process, configuration file and account uses an isolated temporary root.
func TestEngineSmoke(t *testing.T) {
	engine, worker := os.Getenv("III_BINARY"), os.Getenv("WHATSAPP_BINARY")
	if engine == "" || worker == "" {
		t.Skip("set III_BINARY and WHATSAPP_BINARY for isolated loopback integration")
	}
	clearCloudEnvironment(t)
	dir := t.TempDir()
	socket, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	port := socket.Addr().(*net.TCPAddr).Port
	_ = socket.Close()
	engineURL := fmt.Sprintf("ws://127.0.0.1:%d", port)
	engineConfig := filepath.Join(dir, "engine.yaml")
	if err = os.WriteFile(engineConfig, []byte(fmt.Sprintf("workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: %d\n  - name: configuration\n    config:\n      adapter:\n        name: fs\n        config:\n          directory: ./configuration\n", port)), 0600); err != nil {
		t.Fatal(err)
	}
	smokeProcess(t, dir, "engine", engine, []string{"--config", engineConfig, "--no-update-check"}, "III_NAMESPACE=default")
	smokeWait(t, "engine", func() bool {
		_, err := smokeValue(engine, engineURL, "default", "engine::workers::list", map[string]any{})
		return err == nil
	})
	webhookPort := 0
	if httpBinary := os.Getenv("HTTP_BINARY"); httpBinary != "" {
		path := filepath.Join(dir, "http.yaml")
		if err = os.WriteFile(path, []byte("host: 127.0.0.1\nport: 0\nwebhook_listener:\n  host: 127.0.0.1\n  port: 0\n"), 0600); err != nil {
			t.Fatal(err)
		}
		smokeProcess(t, dir, "http", httpBinary, []string{"--url", engineURL, "--config", path}, "III_NAMESPACE="+smokeNamespace, "III_CONFIG_NAME=whatsapp-go-smoke-http", "III_DISABLE_TRACE_PAYLOADS=1")
		smokeWait(t, "HTTP webhook listener", func() bool {
			status, err := smokeTrigger(engine, engineURL, smokeNamespace, "http::webhook-listener::status", map[string]any{})
			if err != nil {
				return false
			}
			applied, _ := status["applied"].(map[string]any)
			if applied["host"] != "127.0.0.1" {
				return false
			}
			p, _ := applied["port"].(float64)
			webhookPort = int(p)
			return webhookPort > 0
		})
	}
	config := DefaultConfig()
	config.DataDir = filepath.Join(dir, "accounts")
	var tunnel *smokeTunnelProvider
	if webhookPort > 0 {
		config.Cloud = cloudTestConfig().Cloud
		tunnel = newSmokeTunnelProvider(t, engine, engineURL)
	}
	seed := filepath.Join(dir, "whatsapp.yaml")
	encoded, _ := json.Marshal(config)
	if err = os.WriteFile(seed, encoded, 0600); err != nil {
		t.Fatal(err)
	}
	smokeProcess(t, dir, "whatsapp", worker, []string{"--url", engineURL, "--config", seed}, "III_NAMESPACE="+smokeNamespace, "III_CONFIG_NAME="+smokeNamespace, "HTTPS_PROXY=http://127.0.0.1:9", "https_proxy=http://127.0.0.1:9", "NO_PROXY=127.0.0.1,localhost", "no_proxy=127.0.0.1,localhost")
	smokeWait(t, "WhatsApp central configuration", func() bool {
		status, err := smokeTrigger(engine, engineURL, smokeNamespace, "whatsapp::config-status", map[string]any{})
		return err == nil && status["id"] == smokeNamespace && status["source"] == "central" && status["last_outcome"] == "applied"
	})
	identity, err := smokeTrigger(engine, engineURL, smokeNamespace, "whatsapp::configuration-id", map[string]any{})
	if err != nil || identity["id"] != smokeNamespace {
		t.Fatalf("config identity: %v %v", identity, err)
	}
	schema, err := smokeTrigger(engine, engineURL, "default", "configuration::schema", map[string]any{"id": smokeNamespace})
	if err != nil {
		t.Fatal(err)
	}
	metadata, _ := schema["metadata"].(map[string]any)
	if metadata["ui_form"] != "whatsapp" {
		t.Fatalf("missing ADE form metadata: %v", metadata)
	}
	for _, path := range []string{"whatsapp/page.js", "whatsapp/styles.css"} {
		asset, err := smokeTrigger(engine, engineURL, smokeNamespace, "whatsapp::ui-content", map[string]any{"path": path})
		if err != nil {
			t.Fatal(err)
		}
		content, _ := asset["content"].(string)
		if len(content) < 100 {
			t.Fatalf("empty asset: %s", path)
		}
	}
	for _, id := range []string{"whatsapp::sessions::create", "whatsapp::messages::list", "whatsapp::cloud::messages::send_text", "whatsapp::cloud::messages::send_template"} {
		info, err := smokeTrigger(engine, engineURL, "default", "engine::functions::info", map[string]any{"function_id": id, "namespace": smokeNamespace})
		if err != nil {
			t.Fatal(err)
		}
		expected := catalogEntries[id]
		if !reflect.DeepEqual(info["request_schema"], expected.RequestFormat) || !reflect.DeepEqual(info["response_schema"], expected.ResponseFormat) {
			t.Fatalf("SDK schema adapter changed %s: %+v", id, info)
		}
	}
	if _, err = smokeValue(engine, engineURL, "default", "whatsapp::sessions::list", map[string]any{}); err == nil {
		t.Fatal("worker functions escaped their namespace")
	}
	created, err := smokeTrigger(engine, engineURL, smokeNamespace, "whatsapp::sessions::create", map[string]any{"session_id": "offline"})
	if err != nil || created["status"] != "stopped" {
		t.Fatalf("create: %v %v", created, err)
	}
	if _, err = smokeTrigger(engine, engineURL, smokeNamespace, "whatsapp::sessions::create", map[string]any{"session_id": "../outside"}); err == nil {
		t.Fatal("invalid session ID passed request validation")
	}
	page, err := smokeTrigger(engine, engineURL, smokeNamespace, "whatsapp::chats::list", map[string]any{"session_id": "offline"})
	if err != nil {
		t.Fatal(err)
	}
	if chats, ok := page["chats"].([]any); !ok || len(chats) != 0 {
		t.Fatalf("fresh session has messages: %v", page)
	}
	if _, err = os.Stat(filepath.Join(config.DataDir, "offline", "device.db")); !os.IsNotExist(err) {
		t.Fatal("offline lifecycle opened native credentials")
	}
	config.MaxSessions = 7
	config.MaxInboxMessages = 250
	encoded, _ = json.Marshal(config)
	var tuned any
	_ = json.Unmarshal(encoded, &tuned)
	if _, err = smokeValue(engine, engineURL, "default", "configuration::set", map[string]any{"id": smokeNamespace, "value": tuned}); err != nil {
		t.Fatal(err)
	}
	smokeWait(t, "authoritative live reload", func() bool {
		status, err := smokeTrigger(engine, engineURL, smokeNamespace, "whatsapp::config-status", map[string]any{})
		if err != nil {
			return false
		}
		active, _ := status["active"].(map[string]any)
		return active["max_sessions"] == float64(7) && active["max_inbox_messages"] == float64(250) && status["last_outcome"] == "applied"
	})
	if _, err = smokeValue(engine, engineURL, smokeNamespace, "whatsapp::on-config-change", map[string]any{"value": map[string]any{"max_sessions": 99}}); err != nil {
		t.Fatal(err)
	}
	status, err := smokeTrigger(engine, engineURL, smokeNamespace, "whatsapp::config-status", map[string]any{})
	if err != nil {
		t.Fatal(err)
	}
	active := status["active"].(map[string]any)
	if active["max_sessions"] != float64(7) {
		t.Fatal("reload trusted caller-supplied configuration")
	}
	for _, secret := range []string{config.Cloud.AccessToken, config.Cloud.AppSecret, config.Cloud.VerifyToken} {
		if secret != "" {
			serialized, _ := json.Marshal(status)
			if strings.Contains(string(serialized), secret) {
				t.Fatal("config status exposed a credential")
			}
		}
	}
	if webhookPort > 0 {
		t.Run("cloud_webhook", func(t *testing.T) { smokeCloudWebhook(t, engine, engineURL, webhookPort) })
		t.Run("cloud_events", func(t *testing.T) { smokeCloudEvents(t, engine, engineURL, webhookPort) })
		t.Run("development_tunnel", func(t *testing.T) { smokeDevelopmentTunnel(t, engine, engineURL, tunnel) })
	}
	config.DataDir = filepath.Join(dir, "pending-data-dir")
	encoded, _ = json.Marshal(config)
	_ = json.Unmarshal(encoded, &tuned)
	if _, err = smokeValue(engine, engineURL, "default", "configuration::set", map[string]any{"id": smokeNamespace, "value": tuned}); err != nil {
		t.Fatal(err)
	}
	smokeWait(t, "structural restart marker", func() bool {
		status, err := smokeTrigger(engine, engineURL, smokeNamespace, "whatsapp::config-status", map[string]any{})
		return err == nil && status["last_outcome"] == "restart_required"
	})
	status, _ = smokeTrigger(engine, engineURL, smokeNamespace, "whatsapp::config-status", map[string]any{})
	if status["active"].(map[string]any)["data_dir"] == config.DataDir {
		t.Fatal("live reload moved active storage")
	}
	if _, err = smokeValue(engine, engineURL, smokeNamespace, "whatsapp::sessions::delete", map[string]any{"session_id": "offline"}); err != nil {
		t.Fatal(err)
	}
	t.Run("event_namespace", func(t *testing.T) { smokeEventNamespace(t, engineURL) })
}

func smokeCloudWebhook(t *testing.T, engine, engineURL string, port int) {
	t.Helper()
	status, err := smokeTrigger(engine, engineURL, smokeNamespace, "whatsapp::cloud::status", map[string]any{})
	if err != nil {
		t.Fatal(err)
	}
	path, _ := status["webhook_path"].(string)
	endpoint := fmt.Sprintf("http://127.0.0.1:%d%s", port, path)
	client := &http.Client{Transport: &http.Transport{Proxy: nil}, Timeout: 12 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	t.Cleanup(func() { client.CloseIdleConnections() })
	query := url.Values{"hub.mode": {"subscribe"}, "hub.verify_token": {cloudTestConfig().Cloud.VerifyToken}, "hub.challenge": {"00123456"}}
	smokeWait(t, "Cloud GET route", func() bool {
		response, err := client.Get(endpoint + "?" + query.Encode())
		if err != nil {
			return false
		}
		defer response.Body.Close()
		body, _ := io.ReadAll(response.Body)
		return response.StatusCode == 200 && string(body) == "00123456" && strings.HasPrefix(response.Header.Get("content-type"), "text/plain")
	})
	raw, _ := json.MarshalIndent(cloudTestPayload(cloudTestChange(cloudTestMessage("wamid.smoke"))), "", "  ")
	raw = append(raw, []byte(" \r\n")...)
	mac := hmac.New(sha256.New, []byte(cloudTestConfig().Cloud.AppSecret))
	_, _ = mac.Write(raw)
	signature := "sha256=" + hex.EncodeToString(mac.Sum(nil))
	post := func(body []byte, signed string) int {
		t.Helper()
		req, err := http.NewRequest(http.MethodPost, endpoint, strings.NewReader(string(body)))
		if err != nil {
			t.Fatal(err)
		}
		req.Header.Set("Content-Type", "application/json")
		req.Header.Set("X-Hub-Signature-256", signed)
		response, err := client.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		defer response.Body.Close()
		return response.StatusCode
	}
	if got := post(raw, signature); got != 200 {
		t.Fatalf("signed raw delivery: HTTP %d", got)
	}
	if got := post(raw, signature); got != 200 {
		t.Fatalf("duplicate delivery: HTTP %d", got)
	}
	if got := post(raw, "sha256="+strings.Repeat("0", 64)); got != 403 {
		t.Fatalf("invalid signature: HTTP %d", got)
	}
	page, err := smokeTrigger(engine, engineURL, smokeNamespace, "whatsapp::cloud::messages::list", map[string]any{"chat_id": "+15551234567"})
	if err != nil {
		t.Fatal(err)
	}
	messages, _ := page["messages"].([]any)
	if len(messages) != 1 || messages[0].(map[string]any)["message_id"] != "wamid.smoke" {
		t.Fatalf("webhook persistence: %v", page)
	}
}

func smokeEventNamespace(t *testing.T, engineURL string) {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	provider := iii.New(engineURL, iii.WithName("go-smoke-events"), iii.WithNamespace("go-smoke-producer"))
	defer provider.Close()
	consumer := iii.New(engineURL, iii.WithName("go-smoke-consumer"), iii.WithNamespace("go-smoke-consumer"))
	defer func() {
		// Stop forwarding before closing its target; an invocation response may
		// still be in flight when the handler signals the assertion below.
		cancel()
		_ = consumer.Close()
	}()
	if err := provider.Connect(ctx); err != nil {
		t.Fatal(err)
	}
	if err := consumer.Connect(ctx); err != nil {
		t.Fatal(err)
	}
	config := DefaultConfig()
	config.DataDir = t.TempDir()
	manager, err := NewManager(config)
	if err != nil {
		t.Fatal(err)
	}
	defer manager.Shutdown(context.Background())
	if _, err = manager.Create("offline"); err != nil {
		t.Fatal(err)
	}
	session, err := manager.session("offline")
	if err != nil {
		t.Fatal(err)
	}
	if err = registerEvents(ctx, provider, manager); err != nil {
		t.Fatal(err)
	}
	received := make(chan Event, 1)
	if err = consumer.RegisterFunction("go-smoke::event", func(_ context.Context, raw json.RawMessage) (any, error) {
		var event Event
		if err := json.Unmarshal(raw, &event); err != nil {
			return nil, err
		}
		select {
		case received <- event:
		default:
		}
		return map[string]bool{"ok": true}, nil
	}); err != nil {
		t.Fatal(err)
	}
	filter := json.RawMessage(`{"session_id":"offline","kinds":["message.received"]}`)
	if err = provider.RegisterTriggerNamespaced("go-smoke-event-binding", "whatsapp::events", "go-smoke::event", "go-smoke-consumer", filter); err != nil {
		t.Fatal(err)
	}
	deadline := time.NewTimer(15 * time.Second)
	defer deadline.Stop()
	ticker := time.NewTicker(100 * time.Millisecond)
	defer ticker.Stop()
	for {
		select {
		case event := <-received:
			if event.SessionID != "offline" || event.Kind != "message.received" {
				t.Fatalf("wrong event: %+v", event)
			}
			return
		case <-ticker.C:
			if err := manager.emit(session, "message.received", map[string]any{"synthetic": true}); err != nil {
				t.Fatal(err)
			}
		case <-deadline.C:
			t.Fatal("event did not reach the target namespace")
		}
	}
}
