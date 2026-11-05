package hub

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/coordinator"
	"github.com/iii-hq/workers/mesh/internal/node"
	"github.com/iii-hq/workers/mesh/internal/registry"
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
	timeout := 610 * time.Second
	if fn == "mesh-node::status" {
		timeout = 5 * time.Second
	}
	return client.Call(ctx, iii.CallRequest{FunctionID: name + "::" + fn, Namespace: "default", Payload: raw, Timeout: timeout})
}
func (h *Hub) fetchNodeStatus(ctx context.Context, name string) (node.Status, error) {
	var status node.Status
	raw, err := h.callNode(ctx, name, "mesh-node::status", struct{}{})
	if err != nil {
		return status, err
	}
	if err = json.Unmarshal(raw, &status); err != nil {
		return status, err
	}
	if status.Name != name {
		return status, errors.New("node status identity does not match authenticated route")
	}
	metadata := coordinator.RuntimeMetadata{OS: status.OS, Arch: status.Arch, Version: status.Version, Target: status.Target}
	if err = h.Coordinator.SetRuntimeMetadata(name, metadata); err != nil {
		return status, err
	}
	if err = h.Coordinator.SetOperator(name, status.Operator); err != nil {
		return status, err
	}
	details := NodeDetails{NodeEntry: NodeEntry{Name: name, OS: status.OS, Arch: status.Arch, Version: status.Version, Workers: []ComponentState{}, Drives: []ComponentState{}}, Status: raw}
	for _, p := range status.Peers {
		if p.Name == "hub" {
			details.Path = p.Path
		}
	}
	for _, w := range status.Workers {
		details.Workers = append(details.Workers, ComponentState{Name: w.Name, State: w.State})
	}
	for _, d := range status.Drives {
		details.Drives = append(details.Drives, ComponentState{Name: d.Name, State: d.State})
	}
	h.mu.Lock()
	if h.nodeDetails == nil {
		h.nodeDetails = map[string]NodeDetails{}
	}
	h.nodeDetails[name] = details
	h.mu.Unlock()
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
func (h *Hub) resolveForNode(ctx context.Context, worker, version, target string) (registry.Artifact, error) {
	if target == "" {
		return registry.Artifact{}, node.ErrUnsupportedTarget
	}
	if h.resolveArtifact != nil {
		return h.resolveArtifact(ctx, worker, version, target)
	}
	return registry.Resolve(ctx, nil, "", worker, version, target)
}

func (h *Hub) OnNodeOnline(ctx context.Context, name string) error {
	status, err := h.fetchNodeStatus(ctx, name)
	if err != nil {
		return err
	}
	if status.Target == "" {
		return node.ErrUnsupportedTarget
	}
	if status.Version != h.Version {
		artifact, err := h.resolveForNode(ctx, "mesh", h.Version, status.Target)
		if err != nil {
			return err
		}
		_, err = h.callNode(ctx, name, "mesh-node::update", node.UpdateRequest{Version: artifact.Version, ArtifactURL: artifact.URL, SHA256: artifact.SHA256})
		return err
	}
	cfg := h.Config()
	workers := node.WorkersRequest{Namespace: h.Namespace, Workers: []node.Worker{}}
	resolutionErrors := map[string]error{}
	var applyErrors []error
	for _, spec := range cfg.Nodes[name].Workers {
		worker, version, pinned := strings.Cut(spec, "@")
		if !pinned {
			version = "*"
		}
		var artifact registry.Artifact
		var resolveErr error
		if len(name+"__"+worker) > 64 {
			resolveErr = fmt.Errorf("worker configuration ID exceeds 64 characters: %s", worker)
		} else {
			artifact, resolveErr = h.resolveForNode(ctx, worker, version, status.Target)
		}
		if resolveErr != nil {
			resolutionErrors[worker] = resolveErr
			applyErrors = append(applyErrors, fmt.Errorf("resolve %s: %w", worker, resolveErr))
			workers.Workers = append(workers.Workers, node.Worker{Name: worker, ResolutionError: resolveErr.Error()})
			continue
		}
		workers.Workers = append(workers.Workers, node.Worker{Name: worker, Version: artifact.Version, ArtifactURL: artifact.URL, SHA256: artifact.SHA256})
	}
	raw, err := h.callNode(ctx, name, "mesh-node::workers::apply", workers)
	if err != nil {
		return err
	}
	var results []node.WorkerResult
	if err = json.Unmarshal(raw, &results); err != nil {
		return fmt.Errorf("invalid workers apply result: %w", err)
	}
	for _, result := range results {
		if result.Error != "" && resolutionErrors[result.Name] == nil {
			applyErrors = append(applyErrors, fmt.Errorf("worker %s: %s", result.Name, result.Error))
		}
	}
	raw, err = h.callNode(ctx, name, "mesh-node::drives::apply", h.Coordinator.Desired(name))
	if err != nil {
		applyErrors = append(applyErrors, err)
	} else {
		var results []node.DriveStatus
		if err = json.Unmarshal(raw, &results); err != nil {
			applyErrors = append(applyErrors, fmt.Errorf("invalid drives apply result: %w", err))
		} else {
			for _, r := range results {
				if r.Error != "" {
					applyErrors = append(applyErrors, fmt.Errorf("drive %s: %s", r.Name, r.Error))
				}
			}
		}
	}
	return errors.Join(applyErrors...)
}
