package node

import (
	"context"
	"crypto/ed25519"
	"errors"
	"net/netip"
	"sync"
	"time"

	"github.com/iii-hq/workers/mesh/internal/meshnet"
)

var ErrRemoved = errors.New("node removed from mesh")

type Network interface {
	Self() netip.Addr
	SelfName() (string, bool)
	Peers() []meshnet.PeerStatus
	GatewayPort() (uint16, bool)
	GatewayChanges() <-chan struct{}
	Removed() <-chan struct{}
}

type Runtime struct{ Node, Namespace, URL string }

// SetRuntime reconnects the shared workers when identity, namespace or URL
// changes. An empty URL suspends them until authenticated gateway discovery
// recovers. Controllers must serialize Status with their own mutations.
type WorkerController interface {
	SetRuntime(context.Context, Runtime) error
	Status() []WorkerStatus
}
type Options struct {
	Mesh        Network
	HubIdentity ed25519.PublicKey
	Version     string
	Workers     WorkerController
	// URL reaches the hub gateway (the client's loopback forwarder). It is used
	// only once the gateway is authenticated.
	URL string
}
type Node struct {
	mesh          Network
	verifier      *Verifier
	version       string
	workers       WorkerController
	url           string
	applyMu       sync.Mutex
	statusMu      sync.RWMutex
	wantedVersion string
	namespace     string
}

func New(o Options) (*Node, error) {
	if o.Mesh == nil || o.Workers == nil || len(o.HubIdentity) != ed25519.PublicKeySize || o.Version == "" || o.URL == "" {
		return nil, errors.New("node requires mesh, workers, pinned identity, version and gateway URL")
	}
	return &Node{mesh: o.Mesh, verifier: NewVerifier(o.HubIdentity, time.Minute), version: o.Version, workers: o.Workers, url: o.URL}, nil
}

// WantedVersion is the hub's version while it differs from this node's.
func (n *Node) WantedVersion() string {
	n.statusMu.RLock()
	defer n.statusMu.RUnlock()
	return n.wantedVersion
}
func (n *Node) setRuntime(ctx context.Context, namespace string) error {
	name, ok := n.mesh.SelfName()
	if !ok {
		return errors.New("assigned node name is not available")
	}
	if _, ok = n.mesh.GatewayPort(); !ok {
		return errors.New("authenticated gateway is not available")
	}
	return n.workers.SetRuntime(ctx, Runtime{Node: name, Namespace: namespace, URL: n.url})
}
