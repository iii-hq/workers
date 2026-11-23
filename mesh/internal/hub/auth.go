package hub

import (
	"context"
	"errors"
	"fmt"
	"reflect"
	"slices"
	"strings"
)

type AuthRequest struct {
	Headers     map[string]string   `json:"headers"`
	QueryParams map[string][]string `json:"query_params,omitempty"`
	IPAddress   string              `json:"ip_address,omitempty"`
	// rbac-proxy sends true when it can pin a session to a registration namespace.
	RegistrationNamespace bool `json:"registration_namespace"`
}
type SessionContext struct {
	MeshNode   string `json:"mesh_node"`
	SessionKey string `json:"session_key"`
}
type AuthResult struct {
	RegistrationNamespace string              `json:"registration_namespace"`
	Namespaces            map[string][]string `json:"namespaces"`
	Context               SessionContext      `json:"context"`
}

// Additional call grants never bypass observer confinement.
type GrantDefaults struct{ Allow []string }

func (h *Hub) SetGrantDefaults(defaults GrantDefaults) {
	h.reload.Lock()
	defer h.reload.Unlock()
	if reflect.DeepEqual(h.defaults, defaults) {
		return
	}
	h.defaults = GrantDefaults{Allow: slices.Clone(defaults.Allow)}
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
	// Refused before the ticket is read, so an old proxy that replaced a new one spends no ticket.
	if !req.RegistrationNamespace {
		return AuthResult{}, errors.New("rbac-proxy does not pin registration namespaces: update rbac-proxy")
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
	if h.nameReserved(node) {
		return AuthResult{}, fmt.Errorf("machine name %q is a namespace in use on the hub: set another client.name on the machine", node)
	}
	key, live := h.Sessions.openOwned(node, owner)
	if !live {
		return AuthResult{}, errors.New("mesh connection is closed")
	}
	h.queueNode(node)
	// No grant in the machine's own namespace: rbac-proxy lets a session call, observe and target there only what it registered itself, which the observer hook relies on.
	grants := map[string][]string{h.Namespace: union(h.defaults.Allow, nil)}
	grants["default"] = union(grants["default"], append(configurationCalls(), "configuration::on-updated"))
	return AuthResult{RegistrationNamespace: node, Namespaces: grants, Context: SessionContext{MeshNode: node, SessionKey: key}}, nil
}
func (h *Hub) sessionNode(ctx SessionContext) (string, error) {
	node, ok := h.Sessions.Valid(ctx.SessionKey)
	if !ok || ctx.MeshNode != "" && ctx.MeshNode != node {
		return "", errors.New("invalid mesh session")
	}
	return node, nil
}
