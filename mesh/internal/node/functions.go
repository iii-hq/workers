package node

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/drives"
	"github.com/iii-hq/workers/mesh/internal/meshnet"
)

type Worker struct {
	Name            string `json:"name"`
	Version         string `json:"version"`
	ArtifactURL     string `json:"artifact_url"`
	SHA256          string `json:"sha256"`
	ResolutionError string `json:"resolution_error,omitempty"`
}
type WorkerResult struct {
	Name  string `json:"name"`
	Error string `json:"error,omitempty"`
}
type WorkerStatus struct {
	Name     string `json:"name"`
	Version  string `json:"version"`
	State    string `json:"state"`
	Restarts int    `json:"restarts"`
	LogTail  string `json:"log_tail"`
	Error    string `json:"error,omitempty"`
}
type WorkersRequest struct {
	Namespace string   `json:"namespace"`
	Workers   []Worker `json:"workers"`
}
type DrivesRequest = drives.Desired
type DriveStatus = drives.Result
type Status struct {
	Name          string               `json:"name"`
	Address       string               `json:"address"`
	OS            string               `json:"os"`
	Arch          string               `json:"arch"`
	Target        string               `json:"target"`
	Version       string               `json:"version"`
	Operator      string               `json:"operator"`
	ClockSkewSecs float64              `json:"clock_skew_secs"`
	Peers         []meshnet.PeerStatus `json:"peers"`
	Workers       []WorkerStatus       `json:"workers"`
	Drives        []DriveStatus        `json:"drives"`
}

func (n *Node) Status() Status {
	name, _ := n.mesh.SelfName()
	address := ""
	if ip := n.mesh.Self(); ip.IsValid() {
		address = ip.String()
	}
	s := Status{Name: name, Address: address, OS: n.platform.OS, Arch: n.platform.Arch, Target: n.platform.Target, Version: n.version, Operator: n.operator, ClockSkewSecs: n.verifier.LastSkew().Seconds(), Peers: n.mesh.Peers(), Workers: n.workerStatus(), Drives: []DriveStatus{}}
	if n.drives != nil {
		s.Drives = n.drives.Status()
	}
	if s.Peers == nil {
		s.Peers = []meshnet.PeerStatus{}
	}
	if s.Workers == nil {
		s.Workers = []WorkerStatus{}
	}
	if s.Drives == nil {
		s.Drives = []DriveStatus{}
	}
	return s
}

func (n *Node) workerStatus() []WorkerStatus {
	n.statusMu.RLock()
	failures := append([]WorkerStatus{}, n.workerErrors...)
	n.statusMu.RUnlock()
	failed := make(map[string]bool, len(failures))
	for _, worker := range failures {
		failed[worker.Name] = true
	}
	statuses := []WorkerStatus{}
	for _, worker := range n.workers.Status() {
		if !failed[worker.Name] {
			statuses = append(statuses, worker)
		}
	}
	return append(statuses, failures...)
}

func (n *Node) Register(client *iii.Client) error {
	for _, fn := range []string{"mesh-node::status", "mesh-node::update", "mesh-node::workers::apply", "mesh-node::drives::apply"} {
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
		var request struct{}
		if err = json.Unmarshal(raw, &request); err != nil {
			return nil, err
		}
		result = n.Status()
	case "mesh-node::update":
		var request UpdateRequest
		if err = json.Unmarshal(raw, &request); err != nil {
			return nil, err
		}
		if n.updater == nil {
			n.statusMu.Lock()
			n.wantedVersion = request.Version
			n.statusMu.Unlock()
			return nil, errors.New("client-mode node does not self-update; upgrade the project's mesh worker")
		}
		if err = n.updater.Apply(ctx, request); err != nil {
			return nil, err
		}
		n.restartOnce.Do(func() { close(n.restart) })
		result = map[string]bool{"restarting": true}
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
		runnable := []Worker{}
		failures := []WorkerStatus{}
		failedResults := []WorkerResult{}
		for _, worker := range request.Workers {
			if worker.ResolutionError != "" {
				failures = append(failures, WorkerStatus{Name: worker.Name, State: "error", Error: worker.ResolutionError})
				failedResults = append(failedResults, WorkerResult{Name: worker.Name, Error: worker.ResolutionError})
			} else {
				runnable = append(runnable, worker)
			}
		}
		n.statusMu.Lock()
		n.workerErrors = failures
		n.wantedVersion = "" // the hub only applies workers once the versions match
		n.statusMu.Unlock()
		var results []WorkerResult
		results, err = n.workers.Apply(ctx, runnable)
		result = append(results, failedResults...)
	case "mesh-node::drives::apply":
		var request DrivesRequest
		if err = json.Unmarshal(raw, &request); err != nil {
			return nil, err
		}
		if n.drives == nil {
			result = []DriveStatus{}
			for _, d := range request.Serve {
				result = append(result.([]DriveStatus), DriveStatus{Name: d.Name, State: "unavailable", Error: "drive service is not installed"})
			}
			for _, d := range request.Mount {
				result = append(result.([]DriveStatus), DriveStatus{Name: d.Name, State: "unavailable", Error: "drive service is not installed"})
			}
		} else {
			result, err = n.drives.Apply(ctx, request)
		}
	default:
		return nil, errors.New("unknown node function")
	}
	if err != nil {
		return nil, err
	}
	return json.Marshal(result)
}
