package meshnet

import (
	"context"
	"net"
	"net/netip"
	"time"

	"golang.org/x/net/netutil"
	"tailscale.com/ipn"
)

const peerTag = "tag:p2p" // coordinator.PeerTag

// PeerPort is the one TCP port a node takes from peers other than the hub (coordinator.PeerPort).
const PeerPort = 9136

// A variable so a test can lower it.
// shortcut: eight machines at their own cap of 72 fill the 512, raise it when a real mesh gets there
var peerConns = 512

// SetPeerToPeer asks the hub to show this machine to the others that ask too, or withdraws the request.
func (m *Mesh) SetPeerToPeer(on bool) error {
	edit := &ipn.MaskedPrefs{AdvertiseTagsSet: true}
	if on {
		edit.AdvertiseTags = []string{peerTag}
	}
	_, err := m.lb.EditPrefs(edit)
	return err
}

// ListenPeers takes direct calls on this machine's own address: a real listener gets its flows before the refusal hooks.
func (m *Mesh) ListenPeers() (net.Listener, error) {
	// An address not assigned yet does not parse, so this never binds every address.
	l, err := m.stack.ListenTCP("tcp6", netip.AddrPortFrom(m.Self(), PeerPort).String())
	if err != nil {
		// shortcut: this is "port is in use" while the stack still holds a connection of an earlier listener (60 s after a clean close), so the caller retries; take the flows in GetTCPHandlerForFlow if turning on again must not wait
		return nil, err
	}
	return netutil.LimitListener(l, peerConns), nil
}

// Dial opens a TCP connection inside the mesh; it is an http.Transport DialContext.
func (m *Mesh) Dial(ctx context.Context, _, addr string) (net.Conn, error) {
	to, err := netip.ParseAddrPort(addr)
	if err != nil {
		return nil, err
	}
	ctx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	c, err := m.stack.DialContextTCP(ctx, to)
	if err != nil {
		return nil, err // not c: a nil *gonet.TCPConn in a net.Conn is not nil
	}
	return c, nil
}

// WhoIs names the machine a tailnet connection comes from: WireGuard takes a source address only from the peer the hub's map gives it to.
func (m *Mesh) WhoIs(from netip.AddrPort) (string, bool) {
	n, _, ok := m.lb.WhoIs("tcp", from)
	if !ok {
		return "", false
	}
	return n.Name(), true
}

// HubHeard is when the hub's last map response arrived, a keepalive included: a removal reaches this machine only through that stream.
func (m *Mesh) HubHeard() time.Time {
	if t := m.heard.Load(); t != nil {
		return *t
	}
	return time.Time{}
}

// OnChange sets f to run after each backend notification, a peer leaving the map included, and after each link change or wake of the machine; f must not block.
func (m *Mesh) OnChange(f func()) { m.onChange.Store(&f) }

func (m *Mesh) changed() {
	if f := m.onChange.Load(); f != nil {
		(*f)()
	}
}
