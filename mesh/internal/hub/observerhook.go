package hub

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
)

// Resolved identities are supplied by the proxy from its registration table,
// rather than inferred from a bare ID that may collide with a hub function.
type ObserverHookRequest struct {
	ObserverID               string          `json:"observer_id"`
	FunctionID               string          `json:"function_id"`
	ResolvedFunctionID       string          `json:"resolved_function_id"`
	Namespace                string          `json:"namespace"`
	TargetFunctionID         string          `json:"target_function_id"`
	ResolvedTargetFunctionID string          `json:"resolved_target_function_id"`
	TargetNamespace          string          `json:"target_namespace"`
	Args                     json.RawMessage `json:"args"`
	Filters                  json.RawMessage `json:"filters"`
	Metadata                 json.RawMessage `json:"metadata,omitempty"`
	Context                  SessionContext  `json:"context"`
}
type ObserverHookMapping struct {
	FunctionID       string          `json:"function_id"`
	TargetFunctionID string          `json:"target_function_id"`
	Args             json.RawMessage `json:"args"`
	Filters          json.RawMessage `json:"filters,omitempty"`
}

func (h *Hub) ObserverHook(_ context.Context, req ObserverHookRequest) (ObserverHookMapping, error) {
	node, err := h.sessionNode(req.Context)
	if err != nil {
		return ObserverHookMapping{}, err
	}
	own := func(id string) bool { return strings.HasPrefix(id, node+"::") && len(id) > len(node)+2 }
	deny := func() (ObserverHookMapping, error) {
		return ObserverHookMapping{}, errors.New("observer is outside node scope")
	}
	if req.TargetNamespace != h.Namespace || !own(req.ResolvedTargetFunctionID) {
		return deny()
	}
	var args map[string]json.RawMessage
	if len(req.Args) > 0 && string(req.Args) != "null" && json.Unmarshal(req.Args, &args) != nil {
		return deny()
	}
	// Own sources currently have no supported args; configuration has no args.
	if len(args) != 0 {
		return deny()
	}
	if req.FunctionID == "configuration::on-updated" && req.ResolvedFunctionID == req.FunctionID {
		if req.Namespace != "default" {
			return deny()
		}
		var filters map[string]map[string]json.RawMessage
		if json.Unmarshal(req.Filters, &filters) != nil {
			return deny()
		}
		idOp := filters["input.id"]
		var id string
		if len(idOp) != 1 || json.Unmarshal(idOp["$eq"], &id) != nil || !ownsConfiguration(node, id) {
			return deny()
		}
		for key, op := range filters {
			if key == "input.id" {
				continue
			}
			if key != "input.event_type" || len(op) != 1 {
				return deny()
			}
			var event string
			if json.Unmarshal(op["$eq"], &event) == nil {
				if !configurationEvent(event) {
					return deny()
				}
			} else {
				var events []string
				if json.Unmarshal(op["$in"], &events) != nil || len(events) == 0 {
					return deny()
				}
				for _, event := range events {
					if !configurationEvent(event) {
						return deny()
					}
				}
			}
		}
	} else if req.Namespace != h.Namespace || !own(req.ResolvedFunctionID) || forbiddenSource(req.FunctionID) || forbiddenSource(strings.TrimPrefix(req.ResolvedFunctionID, node+"::")) {
		return deny()
	}
	argsOut := req.Args
	if len(argsOut) == 0 || string(argsOut) == "null" {
		argsOut = json.RawMessage(`{}`)
	}
	return ObserverHookMapping{req.FunctionID, req.TargetFunctionID, argsOut, req.Filters}, nil
}
func configurationEvent(event string) bool {
	return event == "configuration:registered" || event == "configuration:updated" || event == "configuration:deleted"
}
func forbiddenSource(id string) bool {
	for _, prefix := range []string{"harness::", "queue::", "http::", "cron::", "state::", "stream::", "ade::", "configuration::", "engine::"} {
		if strings.HasPrefix(id, prefix) {
			return true
		}
	}
	return false
}
