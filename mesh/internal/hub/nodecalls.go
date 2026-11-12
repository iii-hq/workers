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

func (h *Hub) callNode(ctx context.Context, name, fn string, payload any, timeout time.Duration) (json.RawMessage, error) {
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
	return client.Call(ctx, iii.CallRequest{FunctionID: name + "::" + fn, Namespace: "default", Payload: raw, Timeout: timeout})
}
func (h *Hub) fetchNodeStatus(ctx context.Context, name string) (node.Status, error) {
	var status node.Status
	raw, err := h.callNode(ctx, name, "mesh-node::status", node.StatusRequest{HubVersion: h.Version}, 5*time.Second)
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
	versionMoved := h.nodeDetails[name].Version != details.Version
	h.nodeDetails[name] = details
	h.mu.Unlock()
	if moved {
		h.statusChanged() // the page lists these workers
	}
	if versionMoved {
		h.onFunctionsChange() // only a machine on the hub's version shares and calls
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
// lets its bridge publish the shared functions, then the catalog when the node
// holds another revision. A node on another version gets neither.
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
	raw, err := h.callNode(ctx, name, "mesh-node::workers::apply", node.WorkersRequest{Namespace: h.Namespace}, 5*time.Second)
	if err != nil {
		return err
	}
	// The apply's answer, not the status above: a moved namespace clears the node's catalog.
	var applied node.Status
	if err = json.Unmarshal(raw, &applied); err != nil {
		return err
	}
	h.mu.RLock()
	catalog, read := h.shared.catalog, h.shared.rows != nil
	h.mu.RUnlock()
	// Before the first read the hub knows nothing to send; that read queues every node again.
	if !read || applied.Catalog == catalog.Rev {
		return nil
	}
	// shortcut: every revision change resends the whole catalog to every node, as often as one machine moves its functions; send deltas or fold longer when a real mesh feels it.
	_, err = h.callNode(ctx, name, "mesh-node::catalog::apply", catalog, 30*time.Second)
	return err
}
