package hub

import (
	"bufio"
	"bytes"
	"encoding/binary"
	"errors"
	"io"
	"net"
	"sync"
	"time"
)

var proxySignature = []byte("\r\n\r\n\x00\r\nQUIT\n")
var errProxyHeader = errors.New("invalid PROXY protocol header")

const proxyHeaderTimeout = 5 * time.Second

// proxyListener reads a PROXY protocol v2 header from loopback peers while trust()
// holds, so those connections report the visitor's address. Only reconcileTunnel
// makes trust() true: with the header off, a visitor's first bytes could be a forged one.
type proxyListener struct {
	net.Listener
	trust func() bool
}

func (l proxyListener) Accept() (net.Conn, error) {
	c, err := l.Listener.Accept()
	if err != nil || !l.trust() {
		return c, err
	}
	if a, ok := c.RemoteAddr().(*net.TCPAddr); !ok || !a.IP.IsLoopback() {
		return c, nil
	}
	return &proxyConn{Conn: c, r: bufio.NewReader(c)}, nil
}

// shortcut: no CloseWrite, so net/http skips its half-close on error replies; add it if tunnel visitors lose those replies.
type proxyConn struct {
	net.Conn
	r    *bufio.Reader
	once sync.Once
	addr net.Addr
	err  error
}

// header runs once, on the first RemoteAddr or Read: net/http asks for the address
// before it reads, on the connection's own goroutine.
func (c *proxyConn) header() {
	c.once.Do(func() {
		c.Conn.SetReadDeadline(time.Now().Add(proxyHeaderTimeout))
		c.addr, c.err = readProxyHeader(c.r, c.Conn.RemoteAddr())
		c.Conn.SetReadDeadline(time.Time{})
	})
}

func (c *proxyConn) RemoteAddr() net.Addr { c.header(); return c.addr }

// Read goes through r: it may hold payload bytes read past the header.
func (c *proxyConn) Read(b []byte) (int, error) {
	if c.header(); c.err != nil {
		return 0, c.err
	}
	return c.r.Read(b)
}

// readProxyHeader returns the visitor address from a v2 header, or peer when the
// connection does not start with one (a local client) or the sender did not know
// the address. Bytes that are not a header stay buffered in r.
func readProxyHeader(r *bufio.Reader, peer net.Addr) (net.Addr, error) {
	if b, err := r.Peek(len(proxySignature)); err != nil || !bytes.Equal(b, proxySignature) {
		return peer, nil
	}
	var h [16]byte
	if _, err := io.ReadFull(r, h[:]); err != nil {
		return peer, err
	}
	n := int(binary.BigEndian.Uint16(h[14:]))
	if h[12]>>4 != 2 || n > 512 {
		return peer, errProxyHeader
	}
	body := make([]byte, n)
	if _, err := io.ReadFull(r, body); err != nil {
		return peer, err
	}
	switch {
	case h[12] == 0x20:
		return peer, nil // LOCAL: no visitor address
	case h[12] == 0x21 && h[13] == 0x11 && n >= 12:
		return &net.TCPAddr{IP: net.IP(body[:4]), Port: int(binary.BigEndian.Uint16(body[8:]))}, nil
	case h[12] == 0x21 && h[13] == 0x21 && n >= 36:
		return &net.TCPAddr{IP: net.IP(body[:16]), Port: int(binary.BigEndian.Uint16(body[32:]))}, nil
	}
	return peer, errProxyHeader
}
