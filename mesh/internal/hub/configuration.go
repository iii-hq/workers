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
)

type rpc interface {
	Trigger(context.Context, iii.TriggerRequest) (json.RawMessage, error)
}

func configCall(ctx context.Context, client rpc, id string, payload any) (json.RawMessage, error) {
	b, err := json.Marshal(payload)
	if err != nil {
		return nil, err
	}
	return client.Trigger(ctx, iii.TriggerRequest{FunctionID: id, Namespace: "default", Data: b, Timeout: 15 * time.Second})
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
	document["example"] = DefaultConfig()
	payload := map[string]any{"id": id, "name": "Mesh", "description": "Mesh hub networking, nodes and drives", "schema": document, "metadata": map[string]string{"ui_form": "mesh"}, "initial_value": DefaultConfig()}
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
	return ConfigChangeResult{OK: true}, nil
}
func (h *Hub) registerConfiguration(ctx context.Context, client *iii.Client, local bool) error {
	internal := iii.RegisterFunctionOptions{Metadata: json.RawMessage(`{"internal":true,"trace_hidden":true}`)}
	if err := iii.RegisterFunctionTyped(client, "mesh::configuration-id", func(context.Context, struct{}) (ConfigurationIdentity, error) {
		return ConfigurationIdentity{ID: h.ConfigID}, nil
	}, internal); err != nil {
		return err
	}
	if err := iii.RegisterFunctionTyped(client, "mesh::on-config-change", func(ctx context.Context, e ConfigChangeEvent) (ConfigChangeResult, error) {
		if local {
			return ConfigChangeResult{}, errors.New("configuration reload is unavailable in interface mode")
		}
		return h.onConfigChange(ctx, client, e)
	}, internal); err != nil {
		return err
	}
	if local {
		return nil
	}
	binding, _ := json.Marshal(map[string]any{"configuration_id": h.ConfigID, "event_types": []string{"configuration:registered", "configuration:updated", "configuration:deleted"}})
	if err := client.RegisterTriggerNamespaced("mesh-config-"+h.ConfigID, "configuration", "mesh::on-config-change", h.Namespace, binding); err != nil {
		return err
	}
	if h.proxyReady {
		proxyBinding, _ := json.Marshal(map[string]any{"configuration_id": "rbac-proxy", "event_types": []string{"configuration:registered", "configuration:updated", "configuration:deleted"}})
		if err := client.RegisterTriggerNamespaced("mesh-proxy-config", "configuration", "mesh::on-config-change", h.Namespace, proxyBinding); err != nil {
			return err
		}
		h.refreshProxy(ctx)
	}
	_, err := h.onConfigChange(ctx, client, ConfigChangeEvent{ID: h.ConfigID})
	return err
}
