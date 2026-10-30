package hub

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
)

type TriggerHookRequest struct {
	TriggerID           string          `json:"trigger_id"`
	TriggerType         string          `json:"trigger_type"`
	ResolvedTriggerType string          `json:"resolved_trigger_type"`
	FunctionID          string          `json:"function_id"`
	Config              json.RawMessage `json:"config"`
	Metadata            json.RawMessage `json:"metadata,omitempty"`
	Context             SessionContext  `json:"context"`
}
type TriggerHookMapping struct {
	TriggerID   string          `json:"trigger_id"`
	TriggerType string          `json:"trigger_type"`
	FunctionID  string          `json:"function_id"`
	Config      json.RawMessage `json:"config"`
}

func (h *Hub) TriggerHook(_ context.Context, req TriggerHookRequest) (TriggerHookMapping, error) {
	node, err := h.sessionNode(req.Context)
	if err != nil {
		return TriggerHookMapping{}, err
	}
	kind := req.ResolvedTriggerType
	if kind == "" {
		kind = req.TriggerType
	}
	if forbiddenTrigger(req.TriggerType) || forbiddenTrigger(kind) {
		return TriggerHookMapping{}, errors.New("shared trigger type is forbidden")
	}
	var config map[string]json.RawMessage
	if json.Unmarshal(req.Config, &config) != nil {
		return TriggerHookMapping{}, errors.New("invalid trigger config")
	}
	switch {
	case req.TriggerType == "configuration" || kind == "configuration":
		if req.TriggerType != "configuration" || kind != "configuration" {
			return TriggerHookMapping{}, errors.New("configuration type identity mismatch")
		}
		var id string
		if json.Unmarshal(config["configuration_id"], &id) != nil || !ownsConfiguration(node, id) {
			return TriggerHookMapping{}, errors.New("configuration binding is outside node scope")
		}
		for key, value := range config {
			switch key {
			case "configuration_id":
			case "event_types":
				var events []string
				if json.Unmarshal(value, &events) != nil {
					return TriggerHookMapping{}, errors.New("invalid configuration event filter")
				}
			default:
				return TriggerHookMapping{}, errors.New("unknown configuration trigger key")
			}
		}
	case strings.HasPrefix(kind, node+"::") && len(kind) > len(node)+2:
		// Own types have no observed config fields yet; unknown keys fail closed.
		if len(config) != 0 {
			return TriggerHookMapping{}, errors.New("unknown node trigger key")
		}
	default:
		return TriggerHookMapping{}, errors.New("trigger type is not granted")
	}
	return TriggerHookMapping{TriggerID: req.TriggerID, TriggerType: req.TriggerType, FunctionID: req.FunctionID, Config: req.Config}, nil
}

func forbiddenTrigger(kind string) bool {
	return strings.HasPrefix(kind, "harness::hook::") || kind == "queue" || kind == "http" || kind == "cron" || kind == "state" || kind == "stream" || strings.HasPrefix(kind, "state::") || strings.HasPrefix(kind, "stream::")
}
