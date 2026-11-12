package hub

import (
	"context"
	"encoding/json"
	"errors"
	"log"
	"regexp"
	"slices"
	"strings"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	meshclient "github.com/iii-hq/workers/mesh/internal/client"
)

type MiddlewareRequest struct {
	FunctionID string          `json:"function_id"`
	Payload    json.RawMessage `json:"payload"`
	Action     *iii.CallAction `json:"action"`
	Context    SessionContext  `json:"context"`
	Namespace  string          `json:"namespace"`
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
	if req.FunctionID == meshCallID {
		return h.meshCall(ctx, client, node, req)
	}
	metadata, _ := iii.MetadataFromContext(ctx)
	return client.Call(ctx, iii.CallRequest{FunctionID: req.FunctionID, Payload: req.Payload, Metadata: metadata, Action: req.Action, Namespace: req.Namespace, Timeout: 610 * time.Second})
}

// meshCallID is the one grant every node session holds: a call of a shared function, never registered.
const meshCallID = "mesh::call"

// shortcut: a fixed cap of 64 mesh calls in flight per calling machine; make it a setting when a real mesh reaches it.
const maxCallsInFlight = 64

type meshCallRequest struct {
	Owner      string          `json:"owner"` // "hub" or the machine sharing function_id
	FunctionID string          `json:"function_id"`
	Payload    json.RawMessage `json:"payload"`
}

var errMeshChannels = &iii.InvocationError{Code: "channels_unsupported", Message: "mesh calls are request/response only"}

// meshCall runs a catalog function for the machine caller. The target sees caller
// mesh: only this gate knows the calling machine. Every loop through the mesh
// passes here, so the cap also stops recursion between shared workers.
func (h *Hub) meshCall(ctx context.Context, client rpc, caller string, req MiddlewareRequest) (json.RawMessage, error) {
	var call meshCallRequest
	_ = json.Unmarshal(req.Payload, &call)
	h.mu.Lock()
	row, listed := h.shared.rows[call.FunctionID]
	// The version read last, not the catalog's: a machine gets the catalog in the reconcile that reads its version.
	details, known := h.nodeDetails[caller]
	ready := known && h.shared.rows != nil // after a hub restart, until its first reads
	// An action is refused: an enqueued call would run after a revocation.
	allowed := req.Namespace == h.Namespace && req.Action == nil && ready && details.Version == h.Version &&
		listed && row.owner == call.Owner && row.owner != caller &&
		(row.owner != "hub" || slices.Contains(h.config.Share, row.worker)) // an unshared worker leaves before the next read
	busy := allowed && h.inFlight[caller] >= maxCallsInFlight
	if allowed && !busy {
		if h.inFlight == nil {
			h.inFlight = map[string]int{}
		}
		h.inFlight[caller]++
	}
	h.mu.Unlock()
	if !allowed || busy {
		// Bounded and quoted: the caller writes both.
		log.Printf("mesh call: refused %.200q of %.64q for %s (busy %v, ready %v)", call.FunctionID, call.Owner, caller, busy, ready)
		switch {
		case busy:
			return nil, &iii.InvocationError{Code: "mesh_busy", Message: "too many mesh calls in flight from this machine"}
		case !ready: // a mirror kept across a hub restart would otherwise read as unshared
			return nil, &iii.InvocationError{Code: "mesh_offline", Message: "the hub has not read this machine and its shares yet"}
		}
		return nil, &iii.InvocationError{Code: "FORBIDDEN", Message: "function is not shared with this machine"}
	}
	defer func() {
		h.mu.Lock()
		h.inFlight[caller]--
		h.mu.Unlock()
	}()
	// Every session may create channels on the hub engine, and hub rows never pass a bridge.
	if meshclient.HasChannelRefs(call.Payload) {
		log.Printf("mesh call: refused %q of %q for %s: channel refs sent", call.FunctionID, call.Owner, caller)
		return nil, errMeshChannels
	}
	out, err := client.Call(ctx, iii.CallRequest{FunctionID: call.FunctionID, Namespace: h.Namespace, Payload: call.Payload, Timeout: 610 * time.Second})
	if err != nil {
		return nil, err
	}
	if meshclient.HasChannelRefs(out) {
		log.Printf("mesh call: refused %q of %q for %s: channel refs returned", call.FunctionID, call.Owner, caller)
		return nil, errMeshChannels
	}
	return out, nil
}
