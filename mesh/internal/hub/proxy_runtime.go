package hub

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"net/netip"
	"net/url"
	"reflect"
	"strconv"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"

	meshclient "github.com/iii-hq/workers/mesh/internal/client"
)

func (h *Hub) proxyHealth() string {
	h.mu.RLock()
	defer h.mu.RUnlock()
	if h.proxyStatus == "" {
		return "proxy ownership is not configured"
	}
	return h.proxyStatus
}
func (h *Hub) setProxyHealth(status string, active bool) {
	h.mu.Lock()
	h.proxyStatus = status
	h.mu.Unlock()
	if h.MeshGateway != nil {
		h.MeshGateway.SetActive(active)
	}
	if !active {
		h.Sessions.closeAll()
	}
}
func fetchProxyConfig(ctx context.Context, client rpc) (map[string]any, error) {
	raw, err := configCall(ctx, client, "configuration::get", map[string]any{"id": "rbac-proxy", "raw": true})
	if remoteCode(err, "NOT_FOUND") {
		return nil, nil
	}
	if err != nil {
		return nil, callFailed(err)
	}
	var entry map[string]json.RawMessage
	if json.Unmarshal(raw, &entry) != nil || len(entry["value"]) == 0 {
		return nil, errors.New("malformed proxy configuration response")
	}
	var cfg map[string]any
	if err = json.Unmarshal(entry["value"], &cfg); err != nil {
		return nil, fmt.Errorf("invalid proxy configuration: %w", err)
	}
	return cfg, nil
}
func validProxyHooks(cfg map[string]any) bool {
	rbac, ok := cfg["rbac"].(map[string]any)
	if !ok || cfg["middleware_function_id"] != "mesh::middleware" || rbac["on_observer_registration_function_id"] != "mesh::on-observer-registration" {
		return false
	}
	if exposed, ok := rbac["expose_functions"]; ok && exposed != nil {
		list, ok := exposed.([]any)
		if !ok || len(list) != 0 {
			return false
		}
	}
	for _, key := range []string{"on_function_registration_function_id", "on_trigger_type_registration_function_id", "on_trigger_registration_function_id"} {
		if value := rbac[key]; value != nil && value != "" {
			return false
		}
	}
	return true
}

// The caller holds reload so ownership, auth and grant changes are serialized.
func (h *Hub) reconcileProxy(ctx context.Context) (err error) {
	defer func() {
		if err != nil {
			h.setProxyHealth(err.Error(), false)
		}
		if errors.As(err, &transient{}) {
			h.proxyRetry.again(h.wakeProxy)
		} else {
			h.proxyRetry.reset() // done, or an event the hub observes ends this failure
		}
	}()
	if !h.proxyReady || h.proxyClient == nil {
		return errors.New("proxy functions are not registered")
	}
	h.mu.RLock()
	closed := h.closed
	h.mu.RUnlock()
	if closed {
		return errors.New("hub is closed")
	}
	host, port, err := proxyEndpoint(h.Config().ProxyURL)
	if err != nil {
		return err
	}
	current, err := fetchProxyConfig(ctx, h.proxyClient)
	if err != nil {
		return fmt.Errorf("read proxy configuration: %w", err)
	}
	next, write, status := EnsureProxyConfig(current)
	if status != "ok" {
		return errors.New(status)
	}
	if write {
		rbac, _ := current["rbac"].(map[string]any)
		if rbac["auth_function_id"] == "mesh::auth" {
			// A legacy hook upgrade never repairs endpoints or changes ownership.
			if !validProxyHooks(next) || next["host"] != host || next["port"] != float64(port) || next["engine_url"] != h.proxyEngineURL() {
				return errors.New("legacy proxy listener or engine URL does not match Mesh")
			}
		} else {
			next["engine_url"] = h.proxyEngineURL()
			next["host"], next["port"] = host, port
		}
		if current == nil {
			_, err = configCall(ctx, h.proxyClient, "configuration::ensure", map[string]any{"id": "rbac-proxy", "name": "RBAC Proxy", "description": "Mesh gateway proxy", "schema": map[string]any{}, "initial_value": next})
		} else {
			_, err = configCall(ctx, h.proxyClient, "configuration::set", map[string]any{"id": "rbac-proxy", "value": next})
		}
		if err != nil {
			return callFailed(err)
		}
		// Ensure may preserve a concurrent owner; only the authoritative result activates.
		next, err = fetchProxyConfig(ctx, h.proxyClient)
		if err != nil {
			return err
		}
		_, needsWrite, status := EnsureProxyConfig(next)
		if status != "ok" {
			return errors.New(status)
		}
		if needsWrite {
			return errors.New("proxy configuration was not retained")
		}
	}
	if !validProxyHooks(next) {
		return errors.New("proxy mesh hooks or grants are invalid")
	}
	if next["host"] != host || next["port"] != float64(port) {
		return errors.New("proxy listener does not match the configured loopback endpoint")
	}
	if next["engine_url"] != h.proxyEngineURL() {
		return errors.New("proxy engine URL does not match the Hub bus")
	}
	if err := h.applyProxy(ctx, next); err != nil {
		return err
	}
	h.setProxyHealth("ok", true)
	return nil
}
func (h *Hub) registerProxyFunctions(ctx context.Context, url string) error {
	h.reload.Lock()
	defer h.reload.Unlock()
	if h.closed {
		return errors.New("hub is closed")
	}
	internal := iii.RegisterFunctionOptions{Metadata: json.RawMessage(`{"internal":true,"trace_hidden":true}`)}
	if _, err := iii.RegisterFunctionTyped(h.Client, "mesh::auth", h.Auth, internal); err != nil {
		return err
	}
	register := func(client *iii.Client) error {
		forwarded := internal // the response is the called function's result, unchanged
		forwarded.ResponseFormat = json.RawMessage(meshclient.AnyJSON)
		if _, err := iii.RegisterFunctionTyped(client, "mesh::middleware", func(ctx context.Context, r MiddlewareRequest) (json.RawMessage, error) {
			return h.middleware(ctx, client, r)
		}, forwarded); err != nil {
			return err
		}
		_, err := iii.RegisterFunctionTyped(client, "mesh::on-observer-registration", h.ObserverHook, internal)
		return err
	}
	if err := register(h.Client); err != nil {
		return err
	}
	if h.Namespace != "default" {
		h.DefaultClient = iii.New(url, iii.WithOtel(iii.OtelConfig{Enabled: new(bool)}), iii.WithName("mesh-default"), iii.WithNamespace("default"))
		connectCtx, cancel := context.WithTimeout(ctx, 30*time.Second)
		err := h.DefaultClient.Connect(connectCtx)
		cancel()
		if err != nil {
			return err
		}
		if err = register(h.DefaultClient); err != nil {
			return err
		}
	}
	h.proxyClient = h.Client
	h.proxyReady = true
	return nil
}
func (h *Hub) refreshProxy(ctx context.Context) {
	h.reload.Lock()
	defer h.reload.Unlock()
	if err := h.reconcileProxy(ctx); err != nil {
		log.Printf("mesh proxy: %v", err)
	}
}

// wakeProxy asks the hub's loop for a proxy pass.
func (h *Hub) wakeProxy() {
	select {
	case h.proxyKick <- struct{}{}:
	default:
	}
}

// kickProxy is wakeProxy for an event: it may have ended a failure, so the retry starts over.
func (h *Hub) kickProxy() { h.proxyRetry.reset(); h.wakeProxy() }

func (h *Hub) proxyEngineURL() string {
	if h.Client != nil {
		return h.Client.Address()
	}
	return "ws://127.0.0.1:49134"
}

type proxyApplyResult struct {
	OK            bool           `json:"ok"`
	AppliedConfig map[string]any `json:"applied_config"`
	BoundHost     string         `json:"bound_host"`
	BoundPort     int            `json:"bound_port"`
	Error         string         `json:"error"`
}

func (h *Hub) applyProxy(ctx context.Context, desired map[string]any) error {
	raw, err := h.proxyClient.Call(ctx, iii.CallRequest{FunctionID: "rbac-proxy::on-config-change", Namespace: h.Namespace, Payload: json.RawMessage(`{"id":"rbac-proxy"}`), Timeout: 15 * time.Second})
	if err != nil {
		return fmt.Errorf("apply proxy configuration: %w", callFailed(err))
	}
	var result proxyApplyResult
	if json.Unmarshal(raw, &result) != nil {
		return errors.New("malformed proxy application response")
	}
	if !result.OK || result.Error != "" {
		return transient{fmt.Errorf("proxy configuration was not applied: %s", result.Error)}
	}
	_, write, status := EnsureProxyConfig(result.AppliedConfig)
	if write || status != "ok" || !validProxyHooks(result.AppliedConfig) || !configContains(result.AppliedConfig, desired) {
		return errors.New("proxy applied configuration does not match mesh ownership")
	}
	if result.BoundPort < 1 || result.BoundHost != result.AppliedConfig["host"] || float64(result.BoundPort) != result.AppliedConfig["port"] {
		return transient{errors.New("proxy applied listener does not match configuration")}
	}
	return nil
}

// The proxy fills serde defaults, so require every desired value while allowing
// its additional defaulted fields in the authoritative applied snapshot.
func configContains(applied, desired map[string]any) bool {
	for key, want := range desired {
		if child, ok := want.(map[string]any); ok {
			got, ok := applied[key].(map[string]any)
			if !ok || !configContains(got, child) {
				return false
			}
		} else if !reflect.DeepEqual(applied[key], want) {
			return false
		}
	}
	return true
}

func proxyEndpoint(raw string) (string, int, error) {
	u, err := url.Parse(raw)
	if err != nil {
		return "", 0, errors.New("invalid proxy endpoint")
	}
	address, err := netip.ParseAddr(u.Hostname())
	port, portErr := strconv.Atoi(u.Port())
	if err != nil || !address.IsLoopback() || u.Scheme != "ws" || u.User != nil || u.RawQuery != "" || u.Fragment != "" || (u.Path != "" && u.Path != "/") || portErr != nil || port < 1 || port > 65535 {
		return "", 0, errors.New("proxy endpoint must be direct ws on a numeric loopback address and explicit port")
	}
	return address.String(), port, nil
}
