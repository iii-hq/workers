//go:build e2e

package e2e

import (
	"context"
	"crypto/ed25519"
	"encoding/json"
	"errors"
	"fmt"
	"net/netip"
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	meshclient "github.com/iii-hq/workers/mesh/internal/client"
	"github.com/iii-hq/workers/mesh/internal/hub"
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
	var network node.Network = nodeNetwork{n}
	if n.mesh != nil {
		network = n.mesh
	}
	control, err := node.New(node.Options{Mesh: network, HubIdentity: f.identity.Public().(ed25519.PublicKey), Version: "0.1.0", Workers: workers, URL: n.url})
	if err != nil {
		t.Fatal(err)
	}
	if err = control.Register(connect(t, f.ctx, n.url, "mesh-node", "default")); err != nil {
		t.Fatal(err)
	}
}

// A machine that shares a worker with no functions tells the hub, which reads it at once: no function moves and nothing else announces it.
func TestE2EANodeAnnouncesWhatItShares(t *testing.T) {
	f := newFixture(t, "default")
	const entry = "e2e-alpha"
	cfg := meshclient.Config{HubURL: f.frontURL, Join: "env://MESH_E2E_JOIN", Name: "alpha"}
	mustCall(t, f.admin, "default", "configuration::ensure", map[string]any{"id": entry, "name": entry, "description": entry, "schema": map[string]any{}, "initial_value": cfg})
	var code hub.JoinResult
	if err := json.Unmarshal(mustCall(t, f.admin, f.ns, "mesh::join::code", map[string]any{}), &code); err != nil {
		t.Fatal(err)
	}
	t.Setenv("MESH_E2E_JOIN", code.Code)
	local, dir := connect(t, f.ctx, f.engineURL, "mesh", "alpha-local"), t.TempDir()
	ctx, cancel := context.WithCancel(f.ctx)
	done := make(chan error, 1)
	go func() {
		done <- meshclient.Run(ctx, meshclient.Options{Local: local, Namespace: "alpha-local", Version: "0.1.0", Dir: dir, ConfigID: entry, Fetch: func(ctx context.Context) (string, *meshclient.Config, error) {
			out, err := call(ctx, f.admin, "default", "configuration::get", map[string]string{"id": entry})
			var stored struct{ Value meshclient.Config }
			if err == nil {
				err = json.Unmarshal(out, &stored)
			}
			return "client", &stored.Value, err
		}})
	}()
	t.Cleanup(func() {
		cancel()
		select {
		case <-done:
		case <-time.After(15 * time.Second):
			t.Error("the client did not stop")
		}
	})
	// alpha is what one list shows of the machine; a list also reads every online machine again.
	alpha := func() (hub.NodeEntry, error) {
		out, err := call(f.ctx, f.admin, f.ns, "mesh::nodes::list", struct{}{})
		var nodes []hub.NodeEntry
		if err == nil {
			err = json.Unmarshal(out, &nodes)
		}
		for _, n := range nodes {
			if n.Name == "alpha" && n.Online {
				return n, err
			}
		}
		return hub.NodeEntry{}, errors.Join(err, fmt.Errorf("alpha is not online in %+v", nodes))
	}
	// Registered before the wait: a new function makes the hub read every machine again, which would hide a missing announce.
	events := make(chan struct{}, 32)
	if _, err := iii.RegisterFunctionTyped(f.admin, "audit::status", func(context.Context, json.RawMessage) (bool, error) { events <- struct{}{}; return true, nil }); err != nil {
		t.Fatal(err)
	}
	eventually(t, 60*time.Second, "alpha joins", func() error { _, err := alpha(); return err })
	time.Sleep(7 * time.Second) // past the one look at its path that a join arms (5 s), so the next status event is the share's
	h := observe(t, f.admin, iii.Source{FunctionID: "mesh::on-status-changed", Namespace: f.ns, Deliver: iii.DeliverInput}, "audit::status", "status", true)
	defer h.Dispose(f.ctx)
	event := func(what string) {
		t.Helper()
		select {
		case <-events:
		case <-time.After(5 * time.Second):
			t.Fatal(what)
		}
	}
	event("no current status for a new observer")
	cfg.Share = []string{"idle"}
	mustCall(t, f.admin, "default", "configuration::set", map[string]any{"id": entry, "value": cfg})
	event("the hub did not hear of the share") // before: the next 30 s re-read
	n, err := alpha()
	if err != nil || len(n.Workers) != 1 || n.Workers[0].Name != "idle" {
		t.Fatalf("one list after the event shows %+v: %v", n.Workers, err)
	}
}
