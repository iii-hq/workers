package coordinator

import "time"

type NodeChange struct {
	Name    string
	Event   string
	At      time.Time
	Version string
}

// SetNodeObserver installs a local observer without replaying restored inventory.
// It runs under the state lock to order publication with committed transitions;
// the callback must only enqueue bounded work and must not call the coordinator.
func (c *Coordinator) SetNodeObserver(observer func(NodeChange)) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.nodeObserver = observer
}

func (c *Coordinator) notifyNode(n *node, event string) {
	if c.nodeObserver != nil && n.Name != "hub" {
		c.nodeObserver(NodeChange{Name: n.Name, Event: event, At: c.clock.Now().UTC(), Version: n.Version})
	}
}
