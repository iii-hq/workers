package hub

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"sync"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	meshclient "github.com/iii-hq/workers/mesh/internal/client"
)

type rpc interface {
	Call(context.Context, iii.CallRequest) (json.RawMessage, error)
}

func configCall(ctx context.Context, client rpc, id string, payload any) (json.RawMessage, error) {
	b, err := json.Marshal(payload)
	if err != nil {
		return nil, err
	}
	return client.Call(ctx, iii.CallRequest{FunctionID: id, Namespace: "default", Payload: b, Timeout: 15 * time.Second})
}
func remoteCode(err error, code string) bool {
	var remote *iii.InvocationError
	return errors.As(err, &remote) && remote.Code == code
}

var legacyWarnings sync.Map

func ensureConfig(ctx context.Context, client rpc, id string) (Config, error) {
	schema, err := iii.InferSchema[Config]()
	if err != nil {
		return Config{}, err
	}
	var document map[string]any
	if err = json.Unmarshal(schema, &document); err != nil {
		return Config{}, err
	}
	// parseConfig fills absent fields from DefaultConfig. Compose injects a bare
	// config_override (for example {mode: client}) on first boot, before any value
	// is stored, so the schema must not require hub fields.
	delete(document, "required")
	document["example"] = DefaultConfig()
	payload := map[string]any{"id": id, "name": "Mesh", "description": "Mesh hub and client networking", "schema": document, "metadata": map[string]string{"ui_form": "mesh"}, "initial_value": DefaultConfig()}
	_, err = configCall(ctx, client, "configuration::ensure", payload)
	if err != nil {
		if !remoteCode(err, "function_not_found") {
			return Config{}, fmt.Errorf("ensure mesh config: %w", err)
		}
		if _, warned := legacyWarnings.LoadOrStore(id, true); !warned {
			log.Printf("configuration entry %q: engine lacks configuration::ensure; using non-atomic legacy initialization; upgrade to >=0.24.1 for concurrent-write safety", id)
		}
		raw, e := configCall(ctx, client, "configuration::get", map[string]any{"id": id, "raw": true})
		if e != nil {
			if !remoteCode(e, "NOT_FOUND") {
				return Config{}, e
			}
		} else {
			var stored map[string]json.RawMessage
			if json.Unmarshal(raw, &stored) != nil || len(stored["value"]) == 0 {
				return Config{}, errors.New("malformed configuration response")
			}
			if !bytes.Equal(bytes.TrimSpace(stored["value"]), []byte("null")) {
				delete(payload, "initial_value")
			}
		}
		if _, err = configCall(ctx, client, "configuration::register", payload); err != nil {
			return Config{}, err
		}
	}
	return fetchConfig(ctx, client, id)
}
func fetchConfig(ctx context.Context, client rpc, id string) (Config, error) {
	raw, err := configCall(ctx, client, "configuration::get", map[string]string{"id": id})
	if remoteCode(err, "NOT_FOUND") {
		return DefaultConfig(), nil
	}
	if err != nil {
		return Config{}, err
	}
	var stored struct {
		Value json.RawMessage `json:"value"`
	}
	if err = json.Unmarshal(raw, &stored); err != nil {
		return Config{}, err
	}
	return parseConfig(stored.Value)
}

type ConfigChangeEvent struct {
	ID string `json:"id"`
}
type ConfigChangeResult struct {
	OK bool `json:"ok"`
}
type ConfigurationIdentity struct {
	ID string `json:"id"`
}

func (h *Hub) onConfigChange(ctx context.Context, client rpc, event ConfigChangeEvent) (ConfigChangeResult, error) {
	if event.ID != "" && event.ID != h.ConfigID && event.ID != "rbac-proxy" {
		return ConfigChangeResult{OK: true}, nil
	}
	h.reload.Lock()
	defer h.reload.Unlock()
	if event.ID == "rbac-proxy" {
		err := h.reconcileProxy(ctx)
		return ConfigChangeResult{OK: err == nil}, err
	}
	cfg, err := fetchConfig(ctx, client, h.ConfigID)
	if err != nil {
		return ConfigChangeResult{}, err
	}
	if err = h.applyConfig(ctx, cfg); err != nil {
		return ConfigChangeResult{}, err
	}
	h.statusChanged()
	h.kickTunnel() // not a direct reconcile: this runs under reload
	return ConfigChangeResult{OK: true}, nil
}

// functionsChanged runs every pass that a worker coming or going can change.
func (h *Hub) functionsChanged() { h.onFunctionsChange(); h.kickProxy(); h.kickTunnel() }

// resumed reads again everything an event announces: an event sent while the connection or an observer was away reached no one.
func (h *Hub) resumed(ctx context.Context) {
	if c := h.engineRPC(); c != nil {
		if _, err := h.onConfigChange(ctx, c, ConfigChangeEvent{ID: h.ConfigID}); err != nil {
			log.Printf("mesh configuration: %v", err)
		}
	}
	h.functionsChanged()
}

var internalFunction = iii.RegisterFunctionOptions{Metadata: json.RawMessage(`{"internal":true,"trace_hidden":true}`)}

// registerConfigurationID tells the Console which configuration entry the page edits.
func (h *Hub) registerConfigurationID(client *iii.Client) error {
	_, err := iii.RegisterFunctionTyped(client, "mesh::configuration-id", func(context.Context, struct{}) (ConfigurationIdentity, error) {
		return ConfigurationIdentity{ID: h.ConfigID}, nil
	}, internalFunction)
	return err
}
func (h *Hub) registerConfiguration(ctx context.Context, client *iii.Client, local bool) error {
	internal := internalFunction
	if err := h.registerConfigurationID(client); err != nil {
		return err
	}
	// Observers capture [output]: the configuration event is the Observed envelope's output.
	if _, err := iii.RegisterFunctionTyped(client, "mesh::on-config-change", func(ctx context.Context, e struct {
		Output ConfigChangeEvent `json:"output"`
	}) (ConfigChangeResult, error) {
		if local {
			return ConfigChangeResult{}, errors.New("configuration reload is unavailable in interface mode")
		}
		return h.onConfigChange(ctx, client, e.Output)
	}, internal); err != nil {
		return err
	}
	// Only wakes the Run loop; the observed payload is never trusted as state.
	// Its observer captures [input]: the tunnel event is the Observed envelope's input.
	if _, err := iii.RegisterFunctionTyped(client, "mesh::on-tunnel-change", func(_ context.Context, e struct {
		Input TunnelChangeEvent `json:"input"`
	}) (ConfigChangeResult, error) {
		return h.onTunnelChange(e.Input), nil
	}, internal); err != nil {
		return err
	}
	// Only schedules the passes; the observed payload is never read.
	if _, err := iii.RegisterFunctionTyped(client, "mesh::on-functions-change", func(context.Context, json.RawMessage) (ConfigChangeResult, error) {
		h.functionsChanged()
		return ConfigChangeResult{OK: true}, nil
	}, internal); err != nil {
		return err
	}
	if local {
		return nil
	}
	resumed := func() { h.resumed(ctx) }
	if err := meshclient.Observe(ctx, client, iii.Source{FunctionID: "engine::on-functions-changed", Capture: []iii.Capture{iii.CaptureOutput}}, "mesh::on-functions-change", iii.ObserverOptions{TargetNamespace: h.Namespace}, resumed); err != nil {
		return err
	}
	for _, id := range []string{h.ConfigID, "rbac-proxy"} {
		if id == "rbac-proxy" && !h.proxyReady {
			continue
		}
		if err := meshclient.Observe(ctx, client, iii.Source{FunctionID: "configuration::on-updated", Namespace: "default", Capture: []iii.Capture{iii.CaptureOutput}, Filters: iii.Filters{
			"input.id":         iii.Op{"$eq": id},
			"input.event_type": iii.Op{"$in": []string{"configuration:registered", "configuration:updated", "configuration:deleted"}},
		}}, "mesh::on-config-change", iii.ObserverOptions{ID: "mesh-config-" + id, TargetNamespace: h.Namespace}, resumed); err != nil {
			return err
		}
	}
	if h.proxyReady {
		h.refreshProxy(ctx)
	}
	_, err := h.onConfigChange(ctx, client, ConfigChangeEvent{ID: h.ConfigID})
	return err
}
