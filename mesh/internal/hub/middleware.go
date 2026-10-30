package hub

import (
	"context"
	"encoding/json"
	"errors"
	"regexp"
	"strings"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

type MiddlewareRequest struct {
	FunctionID string             `json:"function_id"`
	Payload    json.RawMessage    `json:"payload"`
	Action     *iii.TriggerAction `json:"action"`
	Context    SessionContext     `json:"context"`
	Namespace  string             `json:"namespace"`
}

var configurationIDPattern = regexp.MustCompile(`^[a-z0-9_-]{1,64}$`)

func ownsConfiguration(node, id string) bool {
	return configurationIDPattern.MatchString(id) && strings.HasPrefix(id, node+"__") && len(id) > len(node)+2
}
func containsTemplate(value any) bool {
	switch v := value.(type) {
	case string:
		return strings.Contains(v, "${")
	case []any:
		for _, item := range v {
			if containsTemplate(item) {
				return true
			}
		}
	case map[string]any:
		for key, item := range v {
			if strings.Contains(key, "${") || containsTemplate(item) {
				return true
			}
		}
	}
	return false
}
func (h *Hub) middleware(ctx context.Context, client rpc, req MiddlewareRequest) (json.RawMessage, error) {
	node, err := h.sessionNode(req.Context)
	if err != nil {
		return nil, err
	}
	if req.FunctionID == "" {
		return nil, errors.New("missing target function")
	}
	if strings.HasPrefix(req.FunctionID, "configuration::") {
		var payload map[string]any
		if json.Unmarshal(req.Payload, &payload) != nil || payload == nil {
			return nil, errors.New("invalid configuration payload")
		}
		id, _ := payload["id"].(string)
		if !ownsConfiguration(node, id) {
			return nil, errors.New("configuration entry is outside node scope")
		}
		switch req.FunctionID {
		case "configuration::get":
			payload["raw"] = true
			req.Payload, err = json.Marshal(payload)
			if err != nil {
				return nil, err
			}
		case "configuration::ensure", "configuration::register", "configuration::set":
			if containsTemplate(payload["value"]) || containsTemplate(payload["initial_value"]) {
				return nil, errors.New("configuration templates are not allowed")
			}
		default:
			return nil, errors.New("configuration function is not allowed")
		}
	}
	if client == nil {
		return nil, errors.New("mesh forwarding is unavailable")
	}
	metadata, _ := iii.MetadataFromContext(ctx)
	return client.Trigger(ctx, iii.TriggerRequest{FunctionID: req.FunctionID, Data: req.Payload, Metadata: metadata, Action: req.Action, Namespace: req.Namespace, Timeout: 610 * time.Second})
}
