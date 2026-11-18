package coordinator

import (
	"testing"

	"tailscale.com/tailcfg"
)

func TestGatewayMetadataAndSTUN(t *testing.T) {
	c := newCoordinator(t, t.TempDir())
	registerNode(t, c, "hub")
	registerNode(t, c, "laptop")
	c.Relay("https://mesh.example:8443")
	c.SetGatewayPort(50123)
	sent, host := map[tailcfg.NodeID]uint64{}, ""
	paired := &tailcfg.DERPNode{Name: "hub", RegionID: region, HostName: "127.0.0.1", DERPPort: 12345, STUNPort: -1}
	r, _ := c.delta(c.byName("laptop"), sent, &host, true, paired)
	if got := string(r.Peers[0].CapMap[GatewayCapability][0]); got != `{"gateway_port":50123}` {
		t.Fatal(got)
	}
	if got := r.DERPMap.Regions[region].Nodes; len(got) != 1 || got[0].STUNPort != -1 {
		t.Fatalf("default relays: %+v", got)
	}
	c.SetGatewayPort(50124)
	c.SetSTUNPort(3479)
	r, _ = c.delta(c.byName("laptop"), sent, &host, false, paired)
	if got := string(r.PeersChanged[0].CapMap[GatewayCapability][0]); got != `{"gateway_port":50124}` {
		t.Fatal(got)
	}
	nodes := r.DERPMap.Regions[region].Nodes
	if len(nodes) != 2 || nodes[0].STUNOnly || !nodes[1].STUNOnly || nodes[1].HostName != "mesh.example" || nodes[1].STUNPort != 3479 || nodes[0].STUNPort != -1 {
		t.Fatalf("paired relays: %+v", nodes)
	}
	if got := c.relayMap().Regions[region].Nodes; len(got) != 1 || got[0].STUNPort != 3479 {
		t.Fatalf("ordinary relay: %+v", got)
	}
	c.SetSTUNPort(0)
	r, _ = c.delta(c.byName("laptop"), sent, &host, false, paired)
	if len(r.DERPMap.Regions[region].Nodes) != 1 {
		t.Fatal("STUN remained enabled")
	}
}

func TestCycleCodePersistsAndKeepsNodes(t *testing.T) {
	dir := t.TempDir()
	c := newCoordinator(t, dir)
	registerNode(t, c, "laptop")
	old := c.Code()
	if err := c.CycleCode(); err != nil {
		t.Fatal(err)
	}
	if old == c.Code() || len(c.Nodes()) != 1 {
		t.Fatal("cycle failed or removed node")
	}
	restored := newCoordinator(t, dir)
	if restored.Code() != c.Code() {
		t.Fatal("code not persisted")
	}
}
