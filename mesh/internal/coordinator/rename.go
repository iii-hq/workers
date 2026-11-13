package coordinator

import (
	"encoding/json"
	"errors"
	"net/http"
	"slices"

	"tailscale.com/types/key"
)

// renamePath is served on the control channel, which authenticates the machine key.
const renamePath = "/machine/rename"

type renameRequest struct {
	NodeKey key.NodePublic `json:"node_key"`
	Name    string         `json:"name"`
}

// renameResponse carries the name the node now has, which may differ from the one it asked for.
type renameResponse struct {
	Name  string `json:"name"`
	Error string `json:"error,omitempty"`
}

// SetBeforeRename installs what runs before a rename picks a name; the hub refreshes the taken function prefixes in it.
func (c *Coordinator) SetBeforeRename(before func() error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.beforeRename = before
}

func (c *Coordinator) serveRename(w http.ResponseWriter, r *http.Request, machine key.MachinePublic) {
	var req renameRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}
	resp := renameResponse{}
	name, err := c.rename(machine, req.NodeKey, req.Name)
	if resp.Name = name; err != nil {
		resp.Error = err.Error()
	}
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(resp)
}

// rename renames only the caller's own node: the control channel proves its machine key.
func (c *Coordinator) rename(machine key.MachinePublic, k key.NodePublic, requested string) (string, error) {
	member := func() *node { // under c.mu
		if n := c.byKey(k); n != nil && n.Machine == machine && c.authorized(n) && n.Name != "hub" {
			return n
		}
		return nil
	}
	c.mu.Lock()
	// Checked before the hub does any work: anyone can open a control channel.
	known, before := member() != nil, c.beforeRename
	c.mu.Unlock()
	if !known {
		return "", errors.New("node keys refused")
	}
	if before != nil {
		if err := before(); err != nil {
			return "", err
		}
	}
	c.mu.Lock()
	n := member() // removed meanwhile?
	if n == nil {
		c.mu.Unlock()
		return "", errors.New("node keys refused")
	}
	old := n.Name
	name := AllocateName(requested, func(s string) bool { return s != old && (c.byName(s) != nil || c.taken != nil && c.taken(s)) })
	if name == old {
		c.mu.Unlock()
		return old, nil
	}
	tokens := c.st.Tokens
	if n.TokenID != "" {
		// validate() requires the creating token to list the node's name.
		for i, t := range tokens {
			if j := slices.Index(t.Nodes, old); t.ID == n.TokenID && j >= 0 {
				c.st.Tokens = slices.Clone(tokens)
				c.st.Tokens[i].Nodes = slices.Clone(t.Nodes)
				c.st.Tokens[i].Nodes[j] = name
			}
		}
	}
	n.Name = name
	// shortcut: no rename throttle, as a joined node can already make the hub save per map request; add one if renames load the disk.
	if err := c.save(); err != nil {
		n.Name, c.st.Tokens = old, tokens
		c.mu.Unlock()
		return "", err
	}
	c.changed(n)
	// Observers follow names: the old one is gone and the new one arrived.
	if c.nodeObserver != nil {
		c.nodeObserver(NodeChange{Name: old, Event: "removed", At: c.clock.Now().UTC(), Version: n.Version})
	}
	c.notifyNode(n, "joined")
	if n.streams > 0 {
		c.notifyNode(n, "online")
	}
	gone := c.gone
	c.mu.Unlock()
	// Not in a goroutine: the old name's sessions close before the node hears its new name.
	if gone != nil {
		gone(old)
	}
	return name, nil
}
