package meshnet

import (
	"bufio"
	"context"
	"crypto/rand"
	"crypto/subtle"
	"io"
	"log"
	"math"
	"net"
	"net/http"
	"net/netip"
	"time"
)

// TokenHeader carries the token Forward returns. A header, not the URL: the SDK
// writes each client's URL to a world-readable marker file, and a browser cannot
// set headers on a WebSocket.
const TokenHeader = "X-Mesh-Forward-Token"

// Forward exposes the hub gateway on a random loopback port for clients that
// can only dial through the OS (the Go SDK takes no dialer). The target is
// resolved per connection, so the URL survives gateway moves. A connection acts
// as this node only when its request carries token in TokenHeader; the header is
// removed before the hub sees the request. The listener closes with the Mesh.
func (m *Mesh) Forward() (url, token string, err error) {
	l, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return "", "", err
	}
	token = rand.Text()
	go func() { <-m.done; l.Close() }()
	go func() {
		for {
			c, err := l.Accept()
			if err != nil {
				return
			}
			go m.forward(c, token)
		}
	}()
	return "ws://" + l.Addr().String(), token, nil
}

func (m *Mesh) forward(local net.Conn, token string) {
	defer local.Close()
	// The token is checked before dialing, so a refused connection reaches nothing.
	local.SetReadDeadline(time.Now().Add(10 * time.Second))
	head := &io.LimitedReader{R: local, N: 64 << 10} // http.ReadRequest has no size limit of its own
	in := bufio.NewReader(head)
	req, err := http.ReadRequest(in)
	if err != nil || subtle.ConstantTimeCompare([]byte(req.Header.Get(TokenHeader)), []byte(token)) != 1 {
		return
	}
	head.N = math.MaxInt64
	local.SetReadDeadline(time.Time{})
	req.Header.Del(TokenHeader)
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
	if err := req.Write(remote); err != nil {
		return
	}
	go func() { io.Copy(remote, in); remote.Close(); local.Close() }()
	io.Copy(local, remote)
}
