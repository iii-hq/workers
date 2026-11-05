package hub

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"net/http/httptest"
	"net/netip"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/gateway"
)

// This boundary needs the companion Rust binary and an engine. Explicit paths
// keep ordinary package tests independent of build outputs and local services.
func TestRunningProxyMeshClaim(t *testing.T) {
	proxyBin := os.Getenv("MESH_TEST_PROXY_BIN")
	if proxyBin == "" {
		t.Skip("set MESH_TEST_PROXY_BIN to run the real engine/proxy boundary")
	}
	engineBin := os.Getenv("III_BIN")
	if engineBin == "" {
		var err error
		engineBin, err = exec.LookPath("iii")
		if err != nil {
			t.Fatal(err)
		}
	}
	ctx, cancel := context.WithTimeout(t.Context(), 30*time.Second)
	defer cancel()
	dir := t.TempDir()
	enginePort, proxyPort := freeProxyTestPort(t), freeProxyTestPort(t)
	engineURL := fmt.Sprintf("ws://127.0.0.1:%d", enginePort)
	proxyURL := fmt.Sprintf("ws://127.0.0.1:%d", proxyPort)
	engineConfig := filepath.Join(dir, "engine.yaml")
	if err := os.WriteFile(engineConfig, []byte(fmt.Sprintf("workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: %d\n", enginePort)), 0600); err != nil {
		t.Fatal(err)
	}
	startProxyTestProcess(t, dir, "engine", engineBin, "-c", engineConfig)
	h := authHub()
	h.Namespace = "default"
	h.config.ProxyURL = proxyURL
	// Extra operator call grants must never widen observer visibility.
	h.config.Nodes["laptop"] = NodeConfig{Allow: []string{"hub::collision", "ade::ui::asset", "state::on-changed"}}
	h.Client = iii.New(engineURL, iii.WithName("mesh"), iii.WithNamespace("default"))
	defer h.Client.Close()
	if err := h.Client.Connect(ctx); err != nil {
		t.Fatal(err)
	}
	seed := filepath.Join(dir, "proxy.yaml")
	if err := os.WriteFile(seed, []byte(fmt.Sprintf("host: 0.0.0.0\nport: %d\nengine_url: %s\n", proxyPort, engineURL)), 0600); err != nil {
		t.Fatal(err)
	}
	startProxyTestProcess(t, dir, "proxy", proxyBin, "--url", engineURL, "--config", seed)
	var initial map[string]any
	waitProxyTest(t, ctx, func() error {
		raw, err := h.Client.Call(ctx, iii.CallRequest{FunctionID: "rbac-proxy::status", Namespace: "default", Payload: json.RawMessage(`{}`), Timeout: time.Second})
		if err == nil {
			err = json.Unmarshal(raw, &initial)
		}
		return err
	})
	if initial["host"] != "0.0.0.0" || initial["port"] != float64(proxyPort) || initial["rbac_enabled"] != false {
		t.Fatalf("proxy did not start default/unowned: %+v", initial)
	}
	h.MeshGateway = gateway.New(func(netip.Addr) (string, bool) { return "laptop", true }, proxyURL, h.Tickets)
	h.SetHooks(Hooks{})
	defer h.Close()
	if err := h.registerProxyFunctions(ctx, engineURL); err != nil {
		t.Fatal(err)
	}
	if err := h.reconcileProxy(ctx); err != nil {
		t.Fatalf("claim running default proxy: %v", err)
	}
	claimed, err := fetchProxyConfig(ctx, h.Client)
	if err != nil {
		t.Fatal(err)
	}
	if claimed["engine_url"] != engineURL || claimed["port"] != float64(proxyPort) {
		t.Fatalf("claim lost actual bus/listener: %+v", claimed)
	}
	gatewayServer := httptest.NewServer(h.MeshGateway)
	defer gatewayServer.Close()
	endpoint := "ws" + strings.TrimPrefix(gatewayServer.URL, "http")
	roundTrip := func(name string) {
		t.Helper()
		node := iii.New(endpoint, iii.WithName(name), iii.WithNamespace("default"))
		defer node.Close()
		if err := node.Connect(ctx); err != nil {
			t.Fatal(err)
		}
		if _, err := iii.RegisterFunctionTyped(node, "echo::ping", func(_ context.Context, p json.RawMessage) (json.RawMessage, error) { return p, nil }); err != nil {
			t.Fatal(err)
		}
		waitProxyTest(t, ctx, func() error {
			out, err := node.Call(ctx, iii.CallRequest{FunctionID: "echo::ping", Namespace: "default", Payload: json.RawMessage(`{"through":"mesh"}`), Timeout: time.Second})
			if err == nil {
				var result map[string]any
				if json.Unmarshal(out, &result) != nil || result["through"] != "mesh" {
					t.Fatalf("unexpected result: %s", out)
				}
			}
			return err
		})
	}
	roundTrip("first")
	checkProxyObservers(t, ctx, h, endpoint)
	// Simulate a persisted pre-observable Mesh-owned block on the same
	// isolated proxy. Only its removed hook should be upgraded.
	legacyRBAC := claimed["rbac"].(map[string]any)
	delete(legacyRBAC, "on_observer_registration_function_id")
	legacyRBAC["on_trigger_registration_function_id"] = "mesh::on-trigger-registration"
	if _, err := configCall(ctx, h.Client, "configuration::set", map[string]any{"id": "rbac-proxy", "value": claimed}); err != nil {
		t.Fatal(err)
	}
	if err := h.reconcileProxy(ctx); err != nil {
		t.Fatalf("upgrade persisted legacy Mesh hook: %v", err)
	}
	claimed, err = fetchProxyConfig(ctx, h.Client)
	if err != nil {
		t.Fatal(err)
	}
	upgradedRBAC := claimed["rbac"].(map[string]any)
	if upgradedRBAC["on_observer_registration_function_id"] != "mesh::on-observer-registration" || upgradedRBAC["on_trigger_registration_function_id"] != nil || claimed["engine_url"] != engineURL || claimed["port"] != float64(proxyPort) {
		t.Fatalf("incorrect legacy upgrade: %+v", claimed)
	}
	blocker, err := net.Listen("tcp4", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer blocker.Close()
	nextPort := blocker.Addr().(*net.TCPAddr).Port
	claimed["port"] = nextPort
	if _, err := configCall(ctx, h.Client, "configuration::set", map[string]any{"id": "rbac-proxy", "value": claimed}); err != nil {
		t.Fatal(err)
	}
	h.config.ProxyURL = fmt.Sprintf("ws://127.0.0.1:%d", nextPort)
	h.MeshGateway.SetProxyURL(h.config.ProxyURL)
	if err := h.reconcileProxy(ctx); err == nil || h.MeshGateway.Status() == "ok" {
		t.Fatal("failed application falsely activated gateway")
	}
	raw, err := h.Client.Call(ctx, iii.CallRequest{FunctionID: "rbac-proxy::status", Namespace: "default", Payload: json.RawMessage(`{}`), Timeout: time.Second})
	if err != nil {
		t.Fatal(err)
	}
	var retained map[string]any
	json.Unmarshal(raw, &retained)
	if retained["port"] != float64(proxyPort) || retained["host"] != "127.0.0.1" || retained["rbac_enabled"] != true {
		t.Fatalf("failed apply lost last-good config/listener: %s", raw)
	}
	before, _ := fetchProxyConfig(ctx, h.Client)
	blocker.Close()
	if err := h.reconcileProxy(ctx); err != nil {
		t.Fatalf("retry unchanged desired config: %v", err)
	}
	after, _ := fetchProxyConfig(ctx, h.Client)
	beforeJSON, _ := json.Marshal(before)
	afterJSON, _ := json.Marshal(after)
	if string(beforeJSON) != string(afterJSON) {
		t.Fatal("recovery rewrote desired config")
	}
	roundTrip("recovered")
}
func freeProxyTestPort(t *testing.T) int {
	t.Helper()
	listener, err := net.Listen("tcp4", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()
	return listener.Addr().(*net.TCPAddr).Port
}
func startProxyTestProcess(t *testing.T, dir, name, binary string, args ...string) {
	t.Helper()
	logPath := filepath.Join(dir, name+".log")
	logFile, err := os.Create(logPath)
	if err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command(binary, args...)
	cmd.Dir = dir
	cmd.Env = append(os.Environ(), "III_NAMESPACE=default", "III_CONFIG_NAME=rbac-proxy")
	cmd.Stdout = logFile
	cmd.Stderr = logFile
	if err := cmd.Start(); err != nil {
		logFile.Close()
		t.Fatal(err)
	}
	t.Cleanup(func() {
		cmd.Process.Kill()
		cmd.Wait()
		logFile.Close()
		if t.Failed() {
			b, _ := os.ReadFile(logPath)
			if len(b) > 6000 {
				b = b[len(b)-6000:]
			}
			t.Logf("%s log: %s", name, b)
		}
	})
}
func waitProxyTest(t *testing.T, ctx context.Context, f func() error) {
	t.Helper()
	for {
		err := f()
		if err == nil {
			return
		}
		select {
		case <-ctx.Done():
			t.Fatalf("%v; last result: %v", ctx.Err(), err)
		case <-time.After(25 * time.Millisecond):
		}
	}
}

func checkProxyObservers(t *testing.T, ctx context.Context, h *Hub, endpoint string) {
	t.Helper()
	// A hub function with the same bare spelling is still a foreign source.
	if _, err := iii.RegisterFunctionTyped(h.Client, "hub::collision", func(_ context.Context, p json.RawMessage) (json.RawMessage, error) { return p, nil }); err != nil {
		t.Fatal(err)
	}
	node := iii.New(endpoint, iii.WithName("observing-node"), iii.WithNamespace("default"))
	defer node.Close()
	if err := node.Connect(ctx); err != nil {
		t.Fatal(err)
	}
	delivered := make(chan json.RawMessage, 16)
	if _, err := iii.RegisterFunctionTyped(node, "echo::received", func(_ context.Context, p json.RawMessage) (ConfigChangeResult, error) {
		delivered <- p
		return ConfigChangeResult{true}, nil
	}); err != nil {
		t.Fatal(err)
	}
	if _, err := iii.RegisterFunctionTyped(node, "echo::event", func(_ context.Context, p json.RawMessage) (json.RawMessage, error) { return p, nil }); err != nil {
		t.Fatal(err)
	}
	// Wait for a call through the proxy to prove these registrations reached iii.
	waitProxyTest(t, ctx, func() error {
		_, err := h.Client.Call(ctx, iii.CallRequest{FunctionID: "laptop::echo::event", Namespace: "default", Payload: json.RawMessage(`{}`), Timeout: time.Second})
		return err
	})
	observe := func(source iii.Source, allow bool) *iii.ObserverHandle {
		t.Helper()
		handle, err := node.Observe(source, "echo::received")
		if err != nil {
			t.Fatal(err)
		}
		ready, cancel := context.WithTimeout(ctx, 3*time.Second)
		defer cancel()
		err = handle.Ready(ready)
		if allow {
			if err != nil {
				t.Fatalf("observe %s: %v", source.FunctionID, err)
			}
		} else {
			var rejected *iii.ObserveRejectionError
			if !errors.As(err, &rejected) || rejected.Code != "FORBIDDEN" {
				t.Fatalf("observer not refused: source=%s err=%v", source.FunctionID, err)
			}
		}
		return handle
	}
	own := observe(iii.Source{FunctionID: "echo::event", Namespace: "default", Deliver: iii.DeliverOutput}, true)
	if _, err := h.Client.Call(ctx, iii.CallRequest{FunctionID: "laptop::echo::event", Namespace: "default", Payload: json.RawMessage(`{"own":true}`), Timeout: time.Second}); err != nil {
		t.Fatal(err)
	}
	select {
	case raw := <-delivered:
		if string(raw) != `{"own":true}` {
			t.Fatalf("own event: %s", raw)
		}
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	own.Dispose(ctx)
	for _, source := range []iii.Source{
		{FunctionID: "hub::collision", Namespace: "default"},
		{FunctionID: "echo::event", Namespace: "foreign"},
		{FunctionID: "desktop::echo::event", Namespace: "default"},
		{FunctionID: "state::on-changed", Namespace: "default"},
		{FunctionID: "ade::ui::asset", Namespace: "default", Args: iii.Args{"path": "mesh/page.js", "kind": "script"}},
		{FunctionID: "configuration::on-updated", Namespace: "default"},
		{FunctionID: "configuration::on-updated", Namespace: "default", Filters: iii.Filters{"input.id": iii.Op{"$eq": "mesh"}}},
		{FunctionID: "configuration::on-updated", Namespace: "foreign", Filters: iii.Filters{"input.id": iii.Op{"$eq": "laptop__echo"}}},
		{FunctionID: "configuration::on-updated", Namespace: "default", Filters: iii.Filters{"input.id": iii.Op{"$in": []string{"laptop__echo", "mesh"}}}},
		{FunctionID: "configuration::on-updated", Namespace: "default", Args: iii.Args{"condition_function_id": "evil"}, Filters: iii.Filters{"input.id": iii.Op{"$eq": "laptop__echo"}}},
	} {
		observe(source, false)
	}
	_, err := configCall(ctx, h.Client, "configuration::ensure", map[string]any{"id": "laptop__echo", "name": "echo", "description": "observer test", "schema": map[string]any{}, "initial_value": map[string]any{}})
	if err != nil {
		t.Fatal(err)
	}
	config := observe(iii.Source{FunctionID: "configuration::on-updated", Namespace: "default", Deliver: iii.DeliverOutput, Filters: iii.Filters{"input.id": iii.Op{"$eq": "laptop__echo"}, "input.event_type": iii.Op{"$eq": "configuration:updated"}}}, true)
	defer config.Dispose(ctx)
	if _, err := configCall(ctx, h.Client, "configuration::set", map[string]any{"id": "laptop__echo", "value": map[string]any{"fresh": true}}); err != nil {
		t.Fatal(err)
	}
	select {
	case raw := <-delivered:
		var event struct {
			ID        string `json:"id"`
			EventType string `json:"event_type"`
		}
		if json.Unmarshal(raw, &event) != nil || event.ID != "laptop__echo" || event.EventType != "configuration:updated" {
			t.Fatalf("configuration event: %s", raw)
		}
	case <-ctx.Done():
		t.Fatal("owned configuration notification missing", ctx.Err())
	}
}
