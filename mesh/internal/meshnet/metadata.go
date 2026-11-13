package meshnet

import "sort"

type PeerStatus struct {
	Name string `json:"name"`
	Path string `json:"path"`
}

// SelfName is the coordinator-assigned name from the authenticated control map.
// Persist it after enrollment; the requested hostname may have been suffixed.
func (m *Mesh) SelfName() (string, bool) {
	if nm := m.lb.NetMapWithPeers(); nm != nil && nm.SelfNode.Valid() && nm.SelfNode.Name() != "" {
		return nm.SelfNode.Name(), true
	}
	return "", false
}
func (m *Mesh) Peers() []PeerStatus {
	peers := []PeerStatus{}
	nm := m.lb.NetMapWithPeers()
	if nm == nil {
		return peers
	}
	status := m.lb.Status()
	for _, p := range nm.Peers {
		path := "idle"
		if s := status.Peer[p.Key()]; s != nil {
			switch {
			case s.CurAddr != "":
				path = "direct " + s.CurAddr
			case s.Active && s.Relay != "":
				path = "relay hub"
			}
		}
		peers = append(peers, PeerStatus{Name: p.Name(), Path: path})
	}
	sort.Slice(peers, func(i, j int) bool { return peers[i].Name < peers[j].Name })
	return peers
}
