//go:build e2e

package e2e

import (
	"context"
	"crypto/ed25519"
	"net/netip"

	"github.com/iii-hq/workers/mesh/internal/meshnet"
	"github.com/iii-hq/workers/mesh/internal/node"
)

type nodeNetwork struct{ n *testNode }

func (n nodeNetwork) Self() netip.Addr {
	ip, _ := n.n.server.TailscaleIPs()
	if ip.IsValid() {
		return ip
	}
	_, ip = n.n.server.TailscaleIPs()
	return ip
}
func (n nodeNetwork) SelfName() (string, bool)        { return n.n.name, true }
func (n nodeNetwork) Peers() []meshnet.PeerStatus     { return nil }
func (n nodeNetwork) GatewayPort() (uint16, bool)     { return 49135, true }
func (n nodeNetwork) GatewayChanges() <-chan struct{} { return nil }
func (n nodeNetwork) Removed() <-chan struct{}        { return nil }

type noWorkers struct{}

func (noWorkers) SetRuntime(context.Context, node.Runtime) error { return nil }
func (noWorkers) Status() []node.WorkerStatus                    { return nil }
func (noWorkers) SetCatalog(node.Catalog)                        {}
func (noWorkers) CatalogRev() string                             { return "" }

func (f *fixture) startNode(n *testNode, workers node.WorkerController) {
	t := f.t
	t.Helper()
	control, err := node.New(node.Options{Mesh: nodeNetwork{n}, HubIdentity: f.identity.Public().(ed25519.PublicKey), Version: "0.1.0", Workers: workers, URL: n.url})
	if err != nil {
		t.Fatal(err)
	}
	if err = control.Register(connect(t, f.ctx, n.url, "mesh-node", "default")); err != nil {
		t.Fatal(err)
	}
}
