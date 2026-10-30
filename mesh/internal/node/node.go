package node

import (
	"context"
	"crypto/ed25519"
	"errors"
	"fmt"
	"net/http"
	"net/netip"
	"os"
	"path/filepath"
	"sync"
	"time"

	"github.com/iii-hq/workers/mesh/internal/meshnet"
)

const DefaultURL = "ws://hub.iii.internal:49135"

var ErrRestartRequired = errors.New("mesh binary updated; restart service")
var ErrRemoved = errors.New("node removed from mesh")
var ErrUnsupportedTarget = errors.New("unsupported node binary target")

type Network interface {
	Self() netip.Addr
	SelfName() (string, bool)
	Peers() []meshnet.PeerStatus
	GatewayPort() (uint16, bool)
	GatewayChanges() <-chan struct{}
	Removed() <-chan struct{}
}

type Runtime struct{ Node, Namespace, URL string }

// SetRuntime restarts desired workers when identity, namespace or URL changes.
// An empty URL suspends them until authenticated gateway discovery recovers.
// Controllers must serialize Status with their own mutations.
type WorkerController interface {
	SetRuntime(context.Context, Runtime) error
	Apply(context.Context, []Worker) ([]WorkerResult, error)
	Status() []WorkerStatus
}
type DriveController interface {
	Apply(context.Context, DrivesRequest) ([]DriveStatus, error)
	Status() []DriveStatus
}
type Options struct {
	Mesh        Network
	HubIdentity ed25519.PublicKey
	Version     string
	Operator    string
	Workers     WorkerController
	Drives      DriveController
	BinaryPath  string
	HTTPClient  *http.Client
}
type Node struct {
	mesh              Network
	verifier          *Verifier
	version, operator string
	platform          Platform
	workers           WorkerController
	drives            DriveController
	updater           *Updater
	applyMu           sync.Mutex
	namespace         string
	restart           chan struct{}
	restartOnce       sync.Once
}

// New borrows the mesh and controllers. The service entrypoint owns stopping
// them after Run returns, including ErrRestartRequired after a verified update.
func New(o Options) (*Node, error) {
	if o.Mesh == nil || o.Workers == nil || len(o.HubIdentity) != ed25519.PublicKeySize || o.Version == "" || o.Operator == "" {
		return nil, errors.New("node requires mesh, workers, pinned identity, version and operator")
	}
	platform := HostPlatform()
	if platform.Target == "" {
		return nil, fmt.Errorf("%w: %s/%s", ErrUnsupportedTarget, platform.OS, platform.Arch)
	}
	if o.BinaryPath == "" {
		path, err := os.Executable()
		if err != nil {
			return nil, err
		}
		o.BinaryPath, err = filepath.EvalSymlinks(path)
		if err != nil {
			return nil, err
		}
	}
	return &Node{mesh: o.Mesh, verifier: NewVerifier(o.HubIdentity, time.Minute), version: o.Version, operator: o.Operator, platform: platform, workers: o.Workers, drives: o.Drives, updater: &Updater{BinaryPath: o.BinaryPath, Client: o.HTTPClient}, restart: make(chan struct{})}, nil
}
func GatewayURL(port uint16) string {
	if port == 0 {
		return DefaultURL
	}
	return fmt.Sprintf("ws://hub.iii.internal:%d", port)
}
func (n *Node) setRuntime(ctx context.Context, namespace string) error {
	name, ok := n.mesh.SelfName()
	if !ok {
		return errors.New("assigned node name is not available")
	}
	port, ok := n.mesh.GatewayPort()
	if !ok {
		return errors.New("authenticated gateway is not available")
	}
	return n.workers.SetRuntime(ctx, Runtime{Node: name, Namespace: namespace, URL: GatewayURL(port)})
}
