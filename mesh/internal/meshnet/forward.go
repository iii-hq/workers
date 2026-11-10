package meshnet

import (
	"context"
	"errors"
	"io"
	"log"
	"net"
	"net/netip"
	"time"
)

// Forward exposes the hub gateway on a random loopback port for clients that
// can only dial through the OS (the Go SDK takes no dialer). The target is
// resolved per connection, so the URL survives gateway moves. Any local process
// that reaches the port acts as this node. The listener closes with the Mesh.
func (m *Mesh) Forward() (string, error) {
	if m.stack == nil {
		return "", errors.New("forward requires a userspace mesh")
	}
	l, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return "", err
	}
	go func() { <-m.done; l.Close() }()
	go func() {
		for {
			c, err := l.Accept()
			if err != nil {
				return
			}
			go m.forward(c)
		}
	}()
	return "ws://" + l.Addr().String(), nil
}

func (m *Mesh) forward(local net.Conn) {
	defer local.Close()
	hub, ok := m.Peer("hub")
	port, up := m.GatewayPort()
	if !ok || !up {
		return // the SDK redials until the netmap advertises the gateway
	}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	remote, err := m.stack.DialContextTCP(ctx, netip.AddrPortFrom(hub, port))
	cancel()
	if err != nil {
		log.Printf("mesh: forward: %v", err)
		return
	}
	defer remote.Close()
	go func() { io.Copy(remote, local); remote.Close(); local.Close() }()
	io.Copy(local, remote)
}
