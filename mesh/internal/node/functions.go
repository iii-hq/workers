package node

import (
	"context"
	"encoding/json"
	"errors"
	"runtime"
	"strings"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/meshnet"
)

type WorkerStatus struct {
	Name      string   `json:"name"`
	State     string   `json:"state"`
	Functions []string `json:"functions"` // local function ids, sorted
}
type WorkersRequest struct {
	Namespace string `json:"namespace"`
}

// StatusRequest carries the hub's version: the hub hands its namespace only to a node on the same one.
type StatusRequest struct {
	HubVersion string `json:"hub_version"`
}
type Status struct {
	Name          string               `json:"name"`
	Address       string               `json:"address"`
	OS            string               `json:"os"`
	Arch          string               `json:"arch"`
	Version       string               `json:"version"`
	ClockSkewSecs float64              `json:"clock_skew_secs"`
	Peers         []meshnet.PeerStatus `json:"peers"`
	Workers       []WorkerStatus       `json:"workers"`
}

func (n *Node) Status() Status {
	name, _ := n.mesh.SelfName()
	address := ""
	if ip := n.mesh.Self(); ip.IsValid() {
		address = ip.String()
	}
	s := Status{Name: name, Address: address, OS: runtime.GOOS, Arch: runtime.GOARCH, Version: n.version, ClockSkewSecs: n.verifier.LastSkew().Seconds(), Peers: n.mesh.Peers(), Workers: n.workers.Status()}
	if s.Peers == nil {
		s.Peers = []meshnet.PeerStatus{}
	}
	if s.Workers == nil {
		s.Workers = []WorkerStatus{}
	}
	return s
}

func (n *Node) Register(client *iii.Client) error {
	for _, fn := range []string{"mesh-node::status", "mesh-node::workers::apply"} {
		if _, err := iii.RegisterFunctionTyped(client, fn, func(ctx context.Context, s Signed) (json.RawMessage, error) { return n.handle(ctx, fn, s) }, iii.RegisterFunctionOptions{Metadata: json.RawMessage(`{"internal":true,"trace_hidden":true}`)}); err != nil {
			return err
		}
	}
	return nil
}
func (n *Node) handle(ctx context.Context, fn string, s Signed) (json.RawMessage, error) {
	raw, err := n.verifier.Verify(fn, s, time.Now())
	if err != nil {
		return nil, err
	}
	var result any
	switch fn {
	case "mesh-node::status":
		var request StatusRequest
		if err = json.Unmarshal(raw, &request); err != nil {
			return nil, err
		}
		wanted := ""
		if request.HubVersion != n.version {
			wanted = request.HubVersion
		}
		n.statusMu.Lock()
		moved := n.wantedVersion != wanted
		n.wantedVersion = wanted
		n.statusMu.Unlock()
		if moved {
			n.changed()
		}
		result = n.Status()
	case "mesh-node::workers::apply":
		var request WorkersRequest
		if err = json.Unmarshal(raw, &request); err != nil {
			return nil, err
		}
		if strings.TrimSpace(request.Namespace) == "" || strings.TrimSpace(request.Namespace) != request.Namespace {
			return nil, errors.New("workers require a namespace")
		}
		n.applyMu.Lock()
		defer n.applyMu.Unlock()
		if err = n.setRuntime(ctx, request.Namespace); err != nil {
			return nil, err
		}
		n.namespace = request.Namespace
		result = struct{}{}
	default:
		return nil, errors.New("unknown node function")
	}
	return json.Marshal(result)
}
