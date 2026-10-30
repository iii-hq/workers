package gateway

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httputil"
	"net/netip"
	"net/url"
	"strings"
	"sync"
	"time"
)

type Gateway struct {
	mu          sync.Mutex
	lookup      func(netip.Addr) (string, bool)
	proxyURL    string
	tickets     *Tickets
	active      bool
	closed      bool
	lastError   string
	connections map[*Connection]string
	servers     map[*http.Server]struct{}
}

func New(lookup func(netip.Addr) (string, bool), proxyURL string, t *Tickets) *Gateway {
	return &Gateway{lookup: lookup, proxyURL: proxyURL, tickets: t, connections: make(map[*Connection]string), servers: make(map[*http.Server]struct{})}
}
func (g *Gateway) Serve(ln net.Listener) error {
	server := &http.Server{Handler: g, ReadHeaderTimeout: 10 * time.Second}
	g.mu.Lock()
	if g.closed {
		g.mu.Unlock()
		ln.Close()
		return http.ErrServerClosed
	}
	g.servers[server] = struct{}{}
	g.mu.Unlock()
	defer func() { g.mu.Lock(); delete(g.servers, server); g.mu.Unlock() }()
	return server.Serve(ln)
}
func (g *Gateway) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	peer, err := netip.ParseAddrPort(r.RemoteAddr)
	if err != nil {
		http.Error(w, "unknown mesh peer", http.StatusForbidden)
		return
	}
	node, ok := g.lookup(peer.Addr().Unmap())
	if !ok || node == "" || node == "hub" {
		http.Error(w, "unknown mesh peer", http.StatusForbidden)
		return
	}
	g.mu.Lock()
	if !g.active || g.closed {
		g.mu.Unlock()
		http.Error(w, "proxy ownership is not active", http.StatusServiceUnavailable)
		return
	}
	target, err := url.Parse(g.proxyURL)
	if err != nil || target.Host == "" {
		g.lastError = "invalid proxy URL"
		g.mu.Unlock()
		http.Error(w, "invalid proxy URL", http.StatusBadGateway)
		return
	}
	switch target.Scheme {
	case "ws":
		target.Scheme = "http"
	case "wss":
		target.Scheme = "https"
	case "http", "https":
	default:
		g.lastError = "invalid proxy URL"
		g.mu.Unlock()
		http.Error(w, "invalid proxy URL", http.StatusBadGateway)
		return
	}
	value, owner := g.tickets.mint(node)
	g.connections[owner] = node
	g.mu.Unlock()
	ctx, cancel := context.WithCancel(r.Context())
	owner.OnClose(cancel)
	defer func() { owner.close(); g.mu.Lock(); delete(g.connections, owner); g.mu.Unlock() }()
	proxy := &httputil.ReverseProxy{
		Rewrite: func(pr *httputil.ProxyRequest) {
			pr.SetURL(target)
			for key := range pr.Out.Header {
				if strings.EqualFold(key, TicketHeader) {
					delete(pr.Out.Header, key)
				}
			}
			pr.Out.Header.Set(TicketHeader, value)
		},
		ModifyResponse: func(response *http.Response) error {
			if response.StatusCode == http.StatusSwitchingProtocols {
				body, ok := response.Body.(io.ReadWriteCloser)
				if !ok {
					return errors.New("proxy upgrade is not writable")
				}
				if !owner.OnClose(func() { body.Close() }) {
					return errors.New("gateway connection closed")
				}
				g.mu.Lock()
				g.lastError = ""
				g.mu.Unlock()
				response.Body = &connectionBody{ReadWriteCloser: body, owner: owner}
			} else {
				g.mu.Lock()
				g.lastError = fmt.Sprintf("proxy did not upgrade (HTTP %d)", response.StatusCode)
				g.mu.Unlock()
				owner.close()
				return errors.New("proxy did not upgrade")
			}
			return nil
		},
		ErrorHandler: func(w http.ResponseWriter, r *http.Request, err error) {
			if ctx.Err() == nil {
				g.mu.Lock()
				g.lastError = "proxy unavailable"
				g.mu.Unlock()
			}
			owner.close()
			http.Error(w, "proxy unavailable", http.StatusBadGateway)
		},
	}
	proxy.ServeHTTP(w, r.WithContext(ctx))
}
func (g *Gateway) CloseConnections() { g.closeMatching(func(string) bool { return true }) }

func (g *Gateway) CloseNode(name string) {
	g.closeMatching(func(node string) bool { return node == name })
}
func (g *Gateway) closeMatching(match func(string) bool) {
	g.mu.Lock()
	var owners []*Connection
	for owner, node := range g.connections {
		if match(node) {
			owners = append(owners, owner)
		}
	}
	g.mu.Unlock()
	for _, owner := range owners {
		owner.close()
	}
}

// SetActive must be enabled only after the proxy owner/auth endpoint is ready.
// Disabling immediately revokes all connections; Close is permanent.
func (g *Gateway) SetActive(active bool) {
	g.mu.Lock()
	g.active = active && !g.closed
	g.mu.Unlock()
	if !active {
		g.closeMatching(func(string) bool { return true })
	}
}

// SetProxyURL invalidates ownership so a changed upstream must be claimed again.
func (g *Gateway) SetProxyURL(proxyURL string) {
	g.mu.Lock()
	g.proxyURL = proxyURL
	g.lastError = ""
	g.active = false
	g.mu.Unlock()
	g.closeMatching(func(string) bool { return true })
}
func (g *Gateway) Status() string {
	g.mu.Lock()
	defer g.mu.Unlock()
	if g.closed {
		return "not running"
	}
	if !g.active {
		return "proxy ownership is not active"
	}
	if g.lastError != "" {
		return g.lastError
	}
	return "ok"
}
func (g *Gateway) Close() error {
	g.mu.Lock()
	g.closed = true
	g.active = false
	var servers []*http.Server
	for s := range g.servers {
		servers = append(servers, s)
	}
	g.mu.Unlock()
	g.closeMatching(func(string) bool { return true })
	for _, s := range servers {
		s.Close()
	}
	return nil
}

// ReverseProxy permits TCP half-close; auth must expire on either copy's EOF.
type connectionBody struct {
	io.ReadWriteCloser
	owner *Connection
}

func (b *connectionBody) Read(p []byte) (int, error) {
	n, err := b.ReadWriteCloser.Read(p)
	if err != nil {
		b.owner.close()
	}
	return n, err
}
func (b *connectionBody) Write(p []byte) (int, error) {
	n, err := b.ReadWriteCloser.Write(p)
	if err != nil {
		b.owner.close()
	}
	return n, err
}
func (b *connectionBody) Close() error {
	err := b.ReadWriteCloser.Close()
	b.owner.close()
	return err
}
