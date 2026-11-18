package coordinator

import "tailscale.com/tailcfg"

const GatewayCapability tailcfg.NodeCapability = "https://iii.dev/mesh/gateway"

// PeerTag is what a node requests in its hostinfo to be shown to the other nodes that request it.
const PeerTag = "tag:p2p"

// PeerPort is the one TCP port a node takes from peers other than the hub; below 16000 so neither the peer API nor an outgoing connection can hold it.
const PeerPort = 9136

// shortcut: peers learn a moved port in the same change that gives the hub node its new filter, so a peer's first SYN may wait one TCP retransmit; announce the port only after the hub's netmap carries it if that second matters.
func (c *Coordinator) SetGatewayPort(port int) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.gatewayPort == port {
		return
	}
	c.gatewayPort = port
	if hub := c.byName("hub"); hub != nil {
		c.changed(hub)
	}
}
func (c *Coordinator) SetSTUNPort(port int) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.stunPort = port
	c.wake()
}
func (c *Coordinator) stun() int {
	if c.stunPort > 0 {
		return c.stunPort
	}
	return -1
}
func (c *Coordinator) CycleCode() error {
	c.mu.Lock()
	defer c.mu.Unlock()
	old := c.st.CodeSeed
	c.st.CodeSeed = newSeed()
	if err := c.save(); err != nil {
		c.st.CodeSeed = old
		return err
	}
	return nil
}
