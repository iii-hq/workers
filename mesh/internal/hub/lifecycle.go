package hub

import (
	"context"
	"crypto/subtle"
	"errors"
	"fmt"
	"log"
	"net"
	"net/http"
	"net/netip"
	"os"
	"path/filepath"
	"sync"
	"sync/atomic"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/coordinator"
	"github.com/iii-hq/workers/mesh/internal/gateway"
	"github.com/iii-hq/workers/mesh/internal/join"
	"github.com/iii-hq/workers/mesh/internal/meshnet"
	"github.com/iii-hq/workers/mesh/internal/relay"
	"tailscale.com/derp/derpserver"
)

// Reload and close callbacks run under the lifecycle lock; they may read
// Config but must not recursively reload or close the hub.
type Hooks struct {
	Gateway       http.Handler
	ProxyStatus   func() string
	RevokeNode    func(string)
	ConfigChanged func(context.Context, Config, Config)
	NodeStatus    func(context.Context, string) (NodeDetails, error)
	Close         func() error
}

type Hub struct {
	events          *nodeEvents
	nodeReconciler  *nodeReconciler
	nodeClient      rpc
	nodeDetails     map[string]NodeDetails
	Coordinator     *coordinator.Coordinator
	MeshGateway     *gateway.Gateway
	Tickets         *gateway.Tickets
	Sessions        *Sessions
	defaults        GrantDefaults
	proxyClient     rpc
	proxyReady      bool
	proxyStatus     string
	Client          *iii.Client
	DefaultClient   *iii.Client
	Version         string
	Namespace       string
	ConfigID        string
	mu              sync.RWMutex
	reload          sync.Mutex
	config          Config
	hooks           Hooks
	closed          bool
	front           *http.Server
	frontListener   net.Listener
	private         *http.Server
	privateURL      string
	admission       *http.Server
	gateway         *http.Server
	gatewayListener net.Listener
	stun            *net.UDPConn
	derp            *derpserver.Server
	shim            *meshnet.Shim
	limit           *join.Limit
	cancel          context.CancelFunc
	tunnelMu        sync.Mutex
	relayMu         sync.Mutex
	tunnel          tunnelState
	tunnelClient    rpc
	tunnelKick      chan struct{}
	refreshing      atomic.Bool  // a re-read of the online nodes is scheduled
	emit            func([]byte) // delivers a mesh::status answer to its observers; nil publishes nothing
	// Set only by reconcileTunnel. tunnelProxy: PROXY v2 headers from loopback are
	// quick-tunnel's. tunnelRaw: the tunnel is OpenTunnel, which hands a visitor's
	// bytes to loopback unchanged, so forwarding headers from loopback are the visitor's.
	tunnelProxy, tunnelRaw atomic.Bool
	// Guarded by mu: what reconcileTunnel last saw.
	tunnelURL, tunnelPhase, tunnelErr string
}

func Start(ctx context.Context, o Options, cfg Config, client *iii.Client) (h *Hub, err error) {
	if err = cfg.Validate(); err != nil {
		return nil, err
	}
	if o.DataDir == "" {
		o.DataDir = filepath.Join(os.Getenv("III_COMPOSE_DIR"), "data", "mesh")
	}
	ctx, cancel := context.WithCancel(ctx)
	h = &Hub{Version: o.Version, Namespace: namespace(), ConfigID: configID(), Client: client, config: cloneConfig(cfg), defaults: GrantDefaults{Allow: DefaultAllow()}, limit: join.NewLimit(10, 10*time.Minute, time.Now), cancel: cancel, tunnelKick: make(chan struct{}, 1)}
	defer func() {
		if err != nil {
			h.Close()
		}
	}()
	h.Coordinator, err = coordinator.New(o.DataDir, func(secret, addr string) (string, error) {
		if h.publicURL() == "" {
			return "", missingPublicURL(h.Config())
		}
		if join.IsTokenSecret(secret) {
			if _, err := h.Coordinator.RedeemToken(secret); err != nil {
				return "", err
			}
		} else if subtle.ConstantTimeCompare([]byte(secret), []byte(join.Secret(h.Coordinator.Identity(), h.Coordinator.Code()))) != 1 {
			return "", join.ErrCode
		}
		if h.Client != nil {
			if err := h.refreshNames(ctx); err != nil {
				return "", err
			}
		}
		return "", nil
	})
	if err != nil {
		return h, err
	}
	h.Coordinator.SetJoinLimit(h.limit)
	h.Sessions = &Sessions{}
	h.Tickets = gateway.NewTickets(10*time.Second, time.Now)
	h.MeshGateway = gateway.New(h.Coordinator.Node, cfg.ProxyURL, h.Tickets)
	h.SetHooks(Hooks{})
	h.Coordinator.SetGone(h.revoke)
	if client != nil {
		if err = h.refreshNames(ctx); err != nil {
			return h, err
		}
		h.Coordinator.SetBeforeRename(func() error { return h.refreshNames(ctx) })
	}
	admissionListener, e := net.Listen("tcp4", "127.0.0.1:0")
	if e != nil {
		return h, e
	}
	h.admission = &http.Server{Handler: relay.AdmitHandler(h.Coordinator.Known), ReadHeaderTimeout: 10 * time.Second}
	go h.admission.Serve(admissionListener)
	h.derp = relay.NewWithKey(h.Coordinator.RelayKey(), "http://"+admissionListener.Addr().String())
	privateListener, e := net.Listen("tcp4", "127.0.0.1:0")
	if e != nil {
		return h, e
	}
	h.privateURL = "http://" + privateListener.Addr().String()
	h.private = &http.Server{Handler: h.frontHandler(), ReadHeaderTimeout: 10 * time.Second}
	go h.private.Serve(privateListener)
	h.frontListener, err = net.Listen("tcp", cfg.Listen)
	if err != nil {
		return h, err
	}
	h.front = &http.Server{Handler: h.frontHandler(), ReadHeaderTimeout: 10 * time.Second}
	go h.front.Serve(proxyListener{h.frontListener, h.tunnelProxy.Load})
	h.applyCoordinator(cfg)
	h.shim, err = meshnet.NewShim(h.privateURL, h.Coordinator.Public())
	if err != nil {
		return h, err
	}
	listeners, e := h.Coordinator.Listen(ctx, h.shim.URL(), cfg.GatewayPort)
	if e != nil {
		return h, e
	}
	h.gatewayListener = listeners[0]
	h.gateway = h.serveGateway(h.gatewayListener)
	if cfg.StunPort > 0 {
		h.stun, err = listenSTUN(cfg.StunPort)
		if err != nil {
			return h, err
		}
	}
	h.Coordinator.SetGatewayPort(cfg.GatewayPort)
	go func() { <-ctx.Done(); h.Close() }()
	return h, nil
}

func (h *Hub) frontHandler() http.Handler {
	mux := http.NewServeMux()
	h.Coordinator.Handle(mux)
	relay.Handle(mux, h.derp)
	mux.Handle("/join", http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		cfg := h.Config()
		if h.publicURL() == "" {
			http.Error(w, missingPublicURL(cfg).Error(), http.StatusServiceUnavailable)
			return
		}
		join.Handler(h.Coordinator.Identity(), h.Coordinator.Code, h.limit, trustedPrefixes(cfg)).ServeHTTP(w, r)
	}))
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		// OpenTunnel hands a visitor's bytes to loopback unchanged, so a forwarding
		// header from a loopback peer is the visitor's, whatever trusted_proxies says.
		if h.tunnelRaw.Load() {
			if peer, err := netip.ParseAddrPort(r.RemoteAddr); err == nil && peer.Addr().Unmap().IsLoopback() {
				r.Header.Del("CF-Connecting-IP")
				r.Header.Del("X-Forwarded-For")
			}
		}
		switch r.URL.Path {
		case "/id", "/ts2021", "/derp", "/derp/probe", "/derp/latency-check", "/join":
			mux.ServeHTTP(w, r)
		default:
			http.NotFound(w, r)
		}
	})
}
func trustedPrefixes(c Config) []netip.Prefix {
	p := make([]netip.Prefix, 0, len(c.TrustedProxies))
	for _, s := range c.TrustedProxies {
		v, _ := netip.ParsePrefix(s)
		p = append(p, v)
	}
	return p
}
func (h *Hub) applyCoordinator(cfg Config) {
	h.Coordinator.SetTrustedProxies(trustedPrefixes(cfg))
	h.syncRelay()
	h.Coordinator.SetSTUNPort(cfg.StunPort)
}
func (h *Hub) Config() Config { h.mu.RLock(); defer h.mu.RUnlock(); return cloneConfig(h.config) }
func (h *Hub) FrontURL() string {
	h.mu.RLock()
	defer h.mu.RUnlock()
	if h.frontListener == nil {
		return ""
	}
	return "http://" + h.frontListener.Addr().String()
}
func (h *Hub) SetHooks(hooks Hooks) {
	changed, revoke, closeHook := hooks.ConfigChanged, hooks.RevokeNode, hooks.Close
	hooks.ConfigChanged = func(ctx context.Context, old, next Config) {
		h.queueNodes()
		if changed != nil {
			changed(ctx, old, next)
		}
	}
	hooks.RevokeNode = func(name string) {
		h.cancelNode(name)
		if revoke != nil {
			revoke(name)
		}
	}
	hooks.Close = func() error {
		h.stopNodeCalls()
		if closeHook != nil {
			return closeHook()
		}
		return nil
	}
	if hooks.NodeStatus == nil && (h.nodeClient != nil || h.Client != nil) {
		hooks.NodeStatus = h.signedNodeDetails
	}

	// Always compose from the supplied bundle, never from previously wrapped hooks.
	if g := h.MeshGateway; g != nil {
		if hooks.Gateway == nil {
			hooks.Gateway = g
		}
		revoke, changed, closeHook := hooks.RevokeNode, hooks.ConfigChanged, hooks.Close
		hooks.RevokeNode = func(name string) {
			g.CloseNode(name)
			h.Sessions.CloseNode(name)
			if revoke != nil {
				revoke(name)
			}
		}
		hooks.ConfigChanged = func(ctx context.Context, old, next Config) {
			if old.GatewayPort != next.GatewayPort {
				g.CloseConnections()
			}
			if old.ProxyURL != next.ProxyURL {
				g.SetProxyURL(next.ProxyURL)
				if h.proxyClient != nil {
					if err := h.reconcileProxy(ctx); err != nil {
						log.Printf("mesh proxy: %v", err)
					}
				}
			}
			if changed != nil {
				changed(ctx, old, next)
			}
		}
		hooks.Close = func() error {
			err := g.Close()
			h.Sessions.closeAll()
			if closeHook != nil {
				err = errors.Join(err, closeHook())
			}
			return err
		}
	}
	if hooks.ProxyStatus == nil {
		hooks.ProxyStatus = h.proxyHealth
	}
	h.mu.Lock()
	defer h.mu.Unlock()
	h.hooks = hooks
}
func (h *Hub) revoke(name string) {
	h.mu.RLock()
	f := h.hooks.RevokeNode
	h.mu.RUnlock()
	if f != nil {
		f(name)
	}
}
func (h *Hub) serveGateway(l net.Listener) *http.Server {
	s := &http.Server{ReadHeaderTimeout: 10 * time.Second, Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		h.mu.RLock()
		handler := h.hooks.Gateway
		h.mu.RUnlock()
		if handler == nil {
			http.Error(w, "gateway is not configured", http.StatusServiceUnavailable)
			return
		}
		handler.ServeHTTP(w, r)
	})}
	go s.Serve(l)
	return s
}

func (h *Hub) ApplyConfig(ctx context.Context, cfg Config) error {
	h.reload.Lock()
	defer h.reload.Unlock()
	return h.applyConfig(ctx, cfg)
}
func (h *Hub) applyConfig(ctx context.Context, cfg Config) error {
	if err := cfg.Validate(); err != nil {
		return err
	}
	h.mu.RLock()
	old, closed := h.config, h.closed
	h.mu.RUnlock()
	if closed {
		return errors.New("hub is closed")
	}
	var front, gateway net.Listener
	var stun *net.UDPConn
	committed := false
	defer func() {
		if !committed {
			if front != nil {
				front.Close()
			}
			if gateway != nil {
				gateway.Close()
			}
			if stun != nil {
				stun.Close()
			}
		}
	}()
	var err error
	if cfg.Listen != old.Listen {
		front, err = net.Listen("tcp", cfg.Listen)
		if err != nil {
			return fmt.Errorf("bind front door: %w", err)
		}
	}
	if cfg.GatewayPort != old.GatewayPort {
		gateway, err = h.Coordinator.ListenTCP(cfg.GatewayPort)
		if err != nil {
			return fmt.Errorf("bind gateway: %w", err)
		}
	}
	if cfg.StunPort != old.StunPort && cfg.StunPort > 0 {
		stun, err = listenSTUN(cfg.StunPort)
		if err != nil {
			return fmt.Errorf("bind STUN: %w", err)
		}
	}
	h.mu.Lock()
	previousFront, previousGateway, previousSTUN := h.front, h.gateway, h.stun
	h.config = cloneConfig(cfg)
	if front != nil {
		h.frontListener = front
		h.front = &http.Server{Handler: h.frontHandler(), ReadHeaderTimeout: 10 * time.Second}
		go h.front.Serve(proxyListener{front, h.tunnelProxy.Load})
	}
	if gateway != nil {
		h.gatewayListener = gateway
		h.gateway = h.serveGateway(gateway)
	}
	if cfg.StunPort != old.StunPort {
		h.stun = stun
	}
	hooks := h.hooks
	h.mu.Unlock()
	committed = true
	if front != nil {
		previousFront.Close()
	}
	if gateway != nil {
		previousGateway.Close()
	}
	if cfg.StunPort != old.StunPort && previousSTUN != nil {
		previousSTUN.Close()
	}
	h.applyCoordinator(cfg)
	h.Coordinator.SetGatewayPort(cfg.GatewayPort)
	if hooks.RevokeNode != nil {
		for _, n := range h.Coordinator.Nodes() {
			if n.Name != "hub" && cfg.ProxyURL != old.ProxyURL {
				hooks.RevokeNode(n.Name)
			}
		}
	}
	if hooks.ConfigChanged != nil {
		hooks.ConfigChanged(ctx, cloneConfig(old), cloneConfig(cfg))
	}
	return nil
}
func (h *Hub) Close() error {
	h.reload.Lock()
	defer h.reload.Unlock()
	h.mu.Lock()
	if h.closed {
		h.mu.Unlock()
		return nil
	}
	h.closed = true
	hooks := h.hooks
	h.mu.Unlock()
	if h.cancel != nil {
		h.cancel()
	}
	var err error
	if h.events != nil {
		if h.Coordinator != nil {
			h.Coordinator.SetNodeObserver(nil)
		}
		h.events.close()
	}
	if hooks.Close != nil {
		err = hooks.Close()
	}
	h.Sessions.closeAll()
	if h.DefaultClient != nil {
		h.DefaultClient.Close()
	}
	if h.gateway != nil {
		h.gateway.Close()
	}
	if h.stun != nil {
		h.stun.Close()
	}
	if h.Coordinator != nil {
		h.Coordinator.Close()
	}
	if h.shim != nil {
		h.shim.Close()
	}
	if h.derp != nil {
		h.derp.Close()
	}
	if h.front != nil {
		h.front.Close()
	}
	if h.private != nil {
		h.private.Close()
	}
	if h.admission != nil {
		h.admission.Close()
	}
	return err
}
