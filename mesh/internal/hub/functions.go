package hub

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"sort"
	"strings"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	meshclient "github.com/iii-hq/workers/mesh/internal/client"
	"github.com/iii-hq/workers/mesh/internal/join"
)

type JoinResult struct {
	Code string `json:"code"`
}

type TokenResult struct {
	TokenID string `json:"token_id"`
	Token   string `json:"token"`
}
type RevokeTokenRequest struct {
	TokenID string `json:"token_id"`
}
type RevokeTokenResult struct {
	Revoked bool `json:"revoked"`
}

func (h *Hub) JoinToken(ctx context.Context, opts join.TokenOptions) (TokenResult, error) {
	h.mu.RLock()
	defer h.mu.RUnlock()
	if _, err := h.joinCode(); err != nil {
		return TokenResult{}, err
	}
	id, token, err := h.Coordinator.IssueToken(h.publicURLLocked(), opts)
	if err != nil {
		return TokenResult{}, err
	}
	return TokenResult{TokenID: id, Token: token}, nil
}
func (h *Hub) RevokeToken(ctx context.Context, req RevokeTokenRequest) (RevokeTokenResult, error) {
	h.mu.RLock()
	defer h.mu.RUnlock()
	if h.Coordinator == nil {
		return RevokeTokenResult{}, errors.New("hub is not running")
	}
	if err := h.Coordinator.RevokeToken(req.TokenID); err != nil {
		return RevokeTokenResult{}, err
	}
	return RevokeTokenResult{Revoked: true}, nil
}

// workerNamespaces lists the namespaces a worker other than that namespace's own machine uses.
// Workers, not function rows. A function row names an unnamed worker after the first segment of its id, and a machine's rows are unnamed for an instant on every connect, because rbac-proxy fixes the namespace before the SDK names the worker. That would reserve the machine's own name.
func workerNamespaces(raw json.RawMessage) (map[string]bool, error) {
	var list struct {
		Workers []struct {
			Name      *string `json:"name"`
			Namespace string  `json:"namespace"`
		} `json:"workers"`
	}
	if err := json.Unmarshal(raw, &list); err != nil {
		return nil, err
	}
	if list.Workers == nil {
		return nil, errors.New("missing engine workers listing")
	}
	taken := map[string]bool{}
	for _, w := range list.Workers {
		if w.Namespace == "" {
			return nil, errors.New("missing engine worker namespace")
		}
		if w.Name == nil || !strings.HasPrefix(*w.Name, "rbac-proxy:"+w.Namespace+"::") {
			taken[w.Namespace] = true
		}
	}
	return taken, nil
}

type NodeRequest struct {
	Name string `json:"name"`
}
type ComponentState struct {
	Name      string   `json:"name"`
	State     string   `json:"state"`
	Functions []string `json:"functions"` // the machine's function ids
}
type NodeEntry struct {
	Name      string           `json:"name"`
	Address   string           `json:"address"`
	OS        string           `json:"os"`
	Arch      string           `json:"arch"`
	Version   string           `json:"version"`
	Online    bool             `json:"online"`
	LastSeen  time.Time        `json:"last_seen"`
	Ephemeral bool             `json:"ephemeral"`
	Path      string           `json:"path"`
	Workers   []ComponentState `json:"workers"`
}
type NodeDetails struct {
	NodeEntry
	Status      json.RawMessage `json:"status"`
	StatusError string          `json:"status_error,omitempty"`
}
type RemoveResult struct {
	Removed bool `json:"removed"`
	JoinResult
}

func (h *Hub) Status() StatusResult {
	h.mu.RLock()
	cfg, closed, hooks := h.config, h.closed, h.hooks
	front, gateway := h.front, h.gateway
	shared := []meshclient.SharedStatus{}
	for _, w := range cfg.Share { // the live list: an unshared worker leaves at once
		if state, ok := h.shared.workers[w]; ok {
			shared = append(shared, state)
		}
	}
	skipped := append([]string{}, h.shared.skipped...)
	h.mu.RUnlock()
	s := StatusResult{Version: h.Version, PublicURL: h.publicURL(), FrontDoor: "not running", Gateway: "gateway is not configured", Proxy: "proxy ownership is not configured", Tunnel: h.tunnelStatus(cfg), Shared: shared, CatalogSkipped: skipped}
	if h.Coordinator == nil {
		return s
	}
	s.Prefix = h.Coordinator.Prefix().String()
	for _, n := range h.Coordinator.Nodes() {
		if n.Name == "hub" {
			s.HubAddress = n.Addr.String()
			continue
		}
		s.Nodes.Total++
		if n.Version != "" && n.Version != h.Version {
			s.Nodes.Outdated++
		}
		if n.Online {
			s.Nodes.Online++
		}
	}
	if !closed && front != nil {
		s.FrontDoor = "ok"
	}
	if hooks.ProxyStatus != nil {
		s.Proxy = hooks.ProxyStatus()
	}
	if !closed && hooks.Gateway != nil && gateway != nil {
		s.Gateway = "ok"
		if status, ok := hooks.Gateway.(interface{ Status() string }); ok {
			s.Gateway = status.Status()
		}
	}
	if closed {
		s.Gateway = "not running"
		s.Proxy = "not running"
	}
	return s
}
func (h *Hub) JoinCode(ctx context.Context, _ struct{}) (JoinResult, error) {
	h.mu.RLock()
	defer h.mu.RUnlock()
	return h.joinCode()
}
func (h *Hub) joinCode() (JoinResult, error) {
	if h.publicURLLocked() == "" {
		return JoinResult{}, missingPublicURL(h.config)
	}
	if h.Coordinator == nil {
		return JoinResult{}, errors.New("hub is not running")
	}
	return JoinResult{Code: h.Coordinator.Code()}, nil
}
func (h *Hub) CycleCode(ctx context.Context, _ struct{}) (JoinResult, error) {
	h.reload.Lock()
	defer h.reload.Unlock()
	h.mu.RLock()
	defer h.mu.RUnlock()
	if _, err := h.joinCode(); err != nil {
		return JoinResult{}, err
	}
	if err := h.Coordinator.CycleCode(); err != nil {
		return JoinResult{}, err
	}
	return h.joinCode()
}
func (h *Hub) RemoveNode(ctx context.Context, r NodeRequest) (RemoveResult, error) {
	h.reload.Lock()
	defer h.reload.Unlock()
	h.mu.RLock()
	defer h.mu.RUnlock()
	if h.Coordinator == nil {
		return RemoveResult{}, errors.New("hub is not running")
	}
	if err := h.Coordinator.Remove(r.Name); err != nil {
		return RemoveResult{}, err
	}
	// A removal needs no hub address; without one there is no code to return.
	result, _ := h.joinCode()
	return RemoveResult{Removed: true, JoinResult: result}, nil
}
func (h *Hub) ListNodes(ctx context.Context, _ struct{}) ([]NodeEntry, error) {
	nodes := []NodeEntry{}
	if h.Coordinator == nil {
		return nodes, nil
	}
	for _, n := range h.Coordinator.Nodes() {
		if n.Name == "hub" {
			continue
		}
		entry := NodeEntry{Name: n.Name, OS: n.OS, Arch: n.Arch, Version: n.Version, Address: n.Addr.String(), Online: n.Online, LastSeen: n.LastSeen, Ephemeral: n.Ephemeral, Workers: []ComponentState{}}
		h.mu.RLock()
		if cached, ok := h.nodeDetails[n.Name]; ok {
			entry.Path = cached.Path
			entry.Workers = append(entry.Workers, cached.Workers...)
		}
		h.mu.RUnlock()
		nodes = append(nodes, entry)
	}
	sort.Slice(nodes, func(i, j int) bool { return nodes[i].Name < nodes[j].Name })
	return nodes, nil
}

// listAndRead is mesh::nodes::list, not ListNodes, which GetNode also calls: a list is the only event for a path that moved.
// shortcut: every list reads every online node again; read the hub backend's own peer status if a large mesh feels it
func (h *Hub) listAndRead(ctx context.Context, in struct{}) ([]NodeEntry, error) {
	h.queueNodes()
	return h.ListNodes(ctx, in)
}
func (h *Hub) GetNode(ctx context.Context, r NodeRequest) (NodeDetails, error) {
	list, _ := h.ListNodes(ctx, struct{}{})
	for _, n := range list {
		if n.Name != r.Name {
			continue
		}
		h.mu.RLock()
		fetch := h.hooks.NodeStatus
		h.mu.RUnlock()
		result := NodeDetails{NodeEntry: n, StatusError: "signed node status is not configured"}
		if fetch != nil {
			remote, err := fetch(ctx, r.Name)
			if err != nil {
				result.StatusError = err.Error()
			} else {
				result.StatusError = ""
				result.Status = remote.Status
				result.OS = remote.OS
				result.Arch = remote.Arch
				result.Version = remote.Version
				result.Path = remote.Path
				result.Workers = remote.Workers
			}
		}
		return result, nil
	}
	return NodeDetails{}, fmt.Errorf("unknown node %q", r.Name)
}

// engineRPC is the hub's own engine connection: engine when a test sets it, else Client.
func (h *Hub) engineRPC() rpc {
	if h.engine != nil {
		return h.engine
	}
	if h.Client != nil {
		return h.Client
	}
	return nil
}
func (h *Hub) refreshNames(ctx context.Context) error {
	// From the read to the last close: a listing read earlier must not replace a later one.
	h.namesMu.Lock()
	defer h.namesMu.Unlock()
	engine := h.engineRPC()
	if engine == nil || h.Coordinator == nil {
		return errors.New("hub engine is not connected")
	}
	raw, err := engine.Call(ctx, iii.CallRequest{FunctionID: "engine::workers::list", Namespace: "default", Payload: json.RawMessage(`{}`), Timeout: 15 * time.Second})
	if err != nil {
		return fmt.Errorf("list engine workers: %w", err)
	}
	taken, err := workerNamespaces(raw)
	if err != nil {
		return err
	}
	taken["default"], taken[h.Namespace] = true, true
	h.mu.Lock()
	h.taken = taken
	h.mu.Unlock()
	h.Coordinator.SetNameTaken(func(name string) bool { return taken[name] })
	// shortcut: engine::workers::list leaves out a connection that sent no pid (a hand-rolled client), and the set is read at hub start, at each join and rename and on each function change, so a project whose workers connect and register nothing is seen at the next function change. Observe engine::on-workers-changed when either gap is seen in practice.
	for _, n := range h.Coordinator.Nodes() {
		if n.Name == "hub" || !taken[n.Name] {
			continue
		}
		log.Printf("mesh: machine %q: another worker uses its namespace on the hub: closing its sessions", n.Name)
		// Only for these machines: CloseNode also voids the admissions in flight for every machine.
		if h.MeshGateway != nil {
			h.MeshGateway.CloseNode(n.Name)
		}
		h.Sessions.CloseNode(n.Name)
	}
	return nil
}

// nameReserved reads the set the last refreshNames stored.
func (h *Hub) nameReserved(name string) bool {
	h.mu.RLock()
	defer h.mu.RUnlock()
	return name == "default" || name == h.Namespace || h.taken[name]
}
func (h *Hub) registerFunctions(ctx context.Context, client *iii.Client) error {
	publish, err := meshclient.StatusEvents(ctx, client, h.statusChanged)
	if err != nil {
		return err
	}
	h.mu.Lock()
	h.emit = publish
	h.mu.Unlock()
	if _, err := iii.RegisterFunctionTyped(client, "mesh::status", func(context.Context, struct{}) (StatusResult, error) { return h.Status(), nil }); err != nil {
		return err
	}
	if _, err := iii.RegisterFunctionTyped(client, "mesh::nodes::list", h.listAndRead); err != nil {
		return err
	}
	if _, err := iii.RegisterFunctionTyped(client, "mesh::nodes::get", h.GetNode); err != nil {
		return err
	}
	if _, err := iii.RegisterFunctionTyped(client, "mesh::nodes::remove", h.RemoveNode); err != nil {
		return err
	}
	if _, err := iii.RegisterFunctionTyped(client, "mesh::join::code", h.JoinCode); err != nil {
		return err
	}
	if _, err := iii.RegisterFunctionTyped(client, "mesh::join::token", h.JoinToken); err != nil {
		return err
	}
	if _, err := iii.RegisterFunctionTyped(client, "mesh::join::revoke", h.RevokeToken); err != nil {
		return err
	}
	_, err = iii.RegisterFunctionTyped(client, "mesh::join::cycle", h.CycleCode)
	return err
}
