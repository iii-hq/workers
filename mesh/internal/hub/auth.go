package hub

import (
	"context"
	"errors"
	"reflect"
	"slices"
	"strings"
)

type AuthRequest struct {
	Headers     map[string]string   `json:"headers"`
	QueryParams map[string][]string `json:"query_params,omitempty"`
	IPAddress   string              `json:"ip_address,omitempty"`
}
type SessionContext struct {
	MeshNode   string `json:"mesh_node"`
	SessionKey string `json:"session_key"`
}
type AuthResult struct {
	FunctionRegistrationPrefix   string              `json:"function_registration_prefix"`
	Namespaces                   map[string][]string `json:"namespaces"`
	AllowTriggerTypeRegistration bool                `json:"allow_trigger_type_registration"`
	AllowedTriggerTypes          []string            `json:"allowed_trigger_types"`
	Context                      SessionContext      `json:"context"`
}

// Empty defaults are deliberate until supported workers have been observed.
// Shared trigger types still require a confinement validator in TriggerHook.
type GrantDefaults struct {
	Allow        []string
	TriggerTypes []string
}

func (h *Hub) SetGrantDefaults(defaults GrantDefaults) {
	h.reload.Lock()
	defer h.reload.Unlock()
	if reflect.DeepEqual(h.defaults, defaults) {
		return
	}
	h.defaults = GrantDefaults{Allow: slices.Clone(defaults.Allow), TriggerTypes: slices.Clone(defaults.TriggerTypes)}
	if h.MeshGateway != nil {
		h.MeshGateway.CloseConnections()
	}
	h.Sessions.closeAll()
}
func configurationCalls() []string {
	return []string{"configuration::get", "configuration::ensure", "configuration::register", "configuration::set"}
}
func union(a, b []string) []string {
	out := append([]string{}, a...)
	for _, s := range b {
		if !slices.Contains(out, s) {
			out = append(out, s)
		}
	}
	return out
}
func (h *Hub) Auth(_ context.Context, req AuthRequest) (AuthResult, error) {
	h.reload.Lock()
	defer h.reload.Unlock()
	if h.Tickets == nil || h.Sessions == nil {
		return AuthResult{}, errors.New("mesh authentication is unavailable")
	}
	var ticket string
	for name, value := range req.Headers {
		if strings.EqualFold(name, "x-iii-mesh-ticket") {
			if ticket != "" {
				return AuthResult{}, errors.New("ambiguous mesh ticket")
			}
			ticket = value
		}
	}
	node, owner, ok := h.Tickets.ConsumeOwned(ticket)
	if !ok {
		return AuthResult{}, errors.New("invalid mesh ticket")
	}
	key, live := h.Sessions.openOwned(node, owner)
	if !live {
		return AuthResult{}, errors.New("mesh connection is closed")
	}
	grants := map[string][]string{h.Namespace: union(h.defaults.Allow, h.Config().Nodes[node].Allow)}
	grants["default"] = union(grants["default"], configurationCalls())
	return AuthResult{FunctionRegistrationPrefix: node, Namespaces: grants, AllowTriggerTypeRegistration: true, AllowedTriggerTypes: union([]string{node + "::*", "configuration"}, h.defaults.TriggerTypes), Context: SessionContext{MeshNode: node, SessionKey: key}}, nil
}
func (h *Hub) sessionNode(ctx SessionContext) (string, error) {
	node, ok := h.Sessions.Valid(ctx.SessionKey)
	if !ok || ctx.MeshNode != "" && ctx.MeshNode != node {
		return "", errors.New("invalid mesh session")
	}
	return node, nil
}
