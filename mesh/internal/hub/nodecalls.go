package hub

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"reflect"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/coordinator"
	"github.com/iii-hq/workers/mesh/internal/node"
)

func (h *Hub) callNode(ctx context.Context, name, fn string, payload any) (json.RawMessage, error) {
	if h.Coordinator == nil {
		return nil, errors.New("hub is not running")
	}
	client := h.nodeClient
	if client == nil && h.Client != nil {
		client = h.Client
	}
	if client == nil {
		return nil, errors.New("node RPC is not configured")
	}
	signed, err := node.Sign(h.Coordinator.Identity(), fn, payload, time.Now())
	if err != nil {
		return nil, err
	}
	raw, err := json.Marshal(signed)
	if err != nil {
		return nil, err
	}
	return client.Call(ctx, iii.CallRequest{FunctionID: name + "::" + fn, Namespace: "default", Payload: raw, Timeout: 5 * time.Second})
}
func (h *Hub) fetchNodeStatus(ctx context.Context, name string) (node.Status, error) {
	var status node.Status
	raw, err := h.callNode(ctx, name, "mesh-node::status", node.StatusRequest{HubVersion: h.Version})
	if err != nil {
		return status, err
	}
	if err = json.Unmarshal(raw, &status); err != nil {
		return status, err
	}
	if status.Name != name {
		return status, errors.New("node status identity does not match authenticated route")
	}
	metadata := coordinator.RuntimeMetadata{OS: status.OS, Arch: status.Arch, Version: status.Version}
	if err = h.Coordinator.SetRuntimeMetadata(name, metadata); err != nil {
		return status, err
	}
	details := NodeDetails{NodeEntry: NodeEntry{Name: name, OS: status.OS, Arch: status.Arch, Version: status.Version, Workers: []ComponentState{}}, Status: raw}
	for _, p := range status.Peers {
		if p.Name == "hub" {
			details.Path = p.Path
		}
	}
	for _, w := range status.Workers {
		c := ComponentState{Name: w.Name, State: w.State, Functions: []string{}}
		for _, id := range w.Functions {
			c.Functions = append(c.Functions, name+"::"+id)
		}
		details.Workers = append(details.Workers, c)
	}
	h.mu.Lock()
	if h.nodeDetails == nil {
		h.nodeDetails = map[string]NodeDetails{}
	}
	moved := !reflect.DeepEqual(h.nodeDetails[name].NodeEntry, details.NodeEntry)
	h.nodeDetails[name] = details
	h.mu.Unlock()
	if moved {
		h.statusChanged() // the page lists these workers
	}
	return status, nil
}
func (h *Hub) signedNodeDetails(ctx context.Context, name string) (NodeDetails, error) {
	if _, err := h.fetchNodeStatus(ctx, name); err != nil {
		return NodeDetails{}, err
	}
	h.mu.RLock()
	defer h.mu.RUnlock()
	return h.nodeDetails[name], nil
}

// OnNodeOnline hands the hub namespace to a node on the hub's version, which
// lets its bridge publish the shared functions. A node on another version gets
// no namespace.
// shortcut: the lock acts only when the namespace is handed out; a node that
// already holds one re-applies it (node/runtime.go refresh) and keeps publishing
// after its hub restarts on another version, until the node restarts. Enforce
// it on the node or at the gateway once builds report different versions.
func (h *Hub) OnNodeOnline(ctx context.Context, name string) error {
	status, err := h.fetchNodeStatus(ctx, name)
	if err != nil {
		return err
	}
	if status.Version != h.Version {
		return fmt.Errorf("node runs mesh %q, the hub runs %q: run one mesh version on both", status.Version, h.Version)
	}
	_, err = h.callNode(ctx, name, "mesh-node::workers::apply", node.WorkersRequest{Namespace: h.Namespace})
	return err
}
