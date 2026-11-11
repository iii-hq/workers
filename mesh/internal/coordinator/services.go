package coordinator

import "tailscale.com/tailcfg"

const GatewayCapability tailcfg.NodeCapability = "https://iii.dev/mesh/gateway"

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
