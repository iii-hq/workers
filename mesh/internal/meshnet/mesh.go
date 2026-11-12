// MIT License
//
// Copyright (c) 2026 Alexander Yue
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

package meshnet

import (
	"context"
	"fmt"
	"log"
	"net"
	"net/netip"
	"os"
	"path/filepath"
	"regexp"
	"sync"
	"sync/atomic"

	"tailscale.com/control/controlclient"
	"tailscale.com/envknob"
	_ "tailscale.com/feature/condregister/portmapper"
	"tailscale.com/health"
	"tailscale.com/ipn"
	"tailscale.com/ipn/ipnauth"
	"tailscale.com/ipn/ipnlocal"
	"tailscale.com/ipn/store"
	"tailscale.com/net/netmon"
	"tailscale.com/net/tsdial"
	"tailscale.com/tsd"
	"tailscale.com/types/key"
	"tailscale.com/types/logger"
	"tailscale.com/types/logid"
	"tailscale.com/types/nettype"
	"tailscale.com/wgengine"
	"tailscale.com/wgengine/netstack"
)

const Removed = "removed"

// refusedKeys is the hub's register answer to a machine it removed (internal/coordinator).
const refusedKeys = "node keys refused"

var nodeName = regexp.MustCompile(`^[a-z][a-z0-9-]{0,30}$`)

type Mesh struct {
	lb             *ipnlocal.LocalBackend
	sys            *tsd.System
	engine         wgengine.Engine
	monitor        *netmon.Monitor
	dialer         *tsdial.Dialer
	health         *health.Tracker
	stack          *netstack.Impl
	shim           *Shim
	unwatch        context.CancelFunc
	watched        chan struct{}
	removed        chan struct{}
	resources      sync.Once
	gateway        atomic.Uint32
	gatewayChanges chan struct{}
	errors         chan string
	done           chan struct{}
}

// UpUserspace joins without root: userspace networking and an OS-chosen UDP port.
func UpUserspace(dir, publicURL, secret, name string, pinned key.MachinePublic) (*Mesh, error) {
	if !nodeName.MatchString(name) {
		return nil, fmt.Errorf("invalid node name %q", name)
	}
	shim, err := NewShim(publicURL, pinned)
	if err != nil {
		return nil, err
	}
	return startMesh(dir, shim, secret, name)
}

// startMesh runs upstream's userspace TUN plus a netstack that only dials out.
func startMesh(dir string, shim *Shim, secret, name string) (_ *Mesh, err error) {
	m := &Mesh{gatewayChanges: make(chan struct{}, 1), errors: make(chan string, 1), done: make(chan struct{}), sys: tsd.NewSystem(), shim: shim, watched: make(chan struct{}), removed: make(chan struct{})}
	watcherStarted := false
	defer func() {
		if err != nil {
			if watcherStarted {
				m.unwatch()
				<-m.watched
			}
			m.release()
		}
	}()
	if err = os.MkdirAll(dir, 0700); err != nil {
		return nil, err
	}
	envknob.Setenv("TS_NO_LOGS_NO_SUPPORT", "true")
	// ipnlocal computes a sockstat directory even with socket logging unavailable.
	envknob.Setenv("TS_LOGS_DIR", dir)
	logf := logger.Discard
	m.monitor, err = netmon.New(m.sys.Bus.Get(), logf)
	if err != nil {
		return nil, err
	}
	m.dialer = &tsdial.Dialer{Logf: logf}
	m.dialer.SetBus(m.sys.Bus.Get())
	m.sys.Set(m.dialer)
	m.engine, err = wgengine.NewUserspaceEngine(logf, wgengine.Config{EventBus: m.sys.Bus.Get(), NetMon: m.monitor, Dialer: m.dialer, SetSubsystem: m.sys.Set, ControlKnobs: m.sys.ControlKnobs(), HealthTracker: m.sys.HealthTracker.Get(), Metrics: m.sys.UserMetricsRegistry()})
	if err != nil {
		return nil, err
	}
	m.sys.Set(m.engine)
	m.health = m.sys.HealthTracker.Get()
	m.health.SetMetricsRegistry(m.sys.UserMetricsRegistry())
	st, err := store.NewFileStore(logf, filepath.Join(dir, "state"))
	if err != nil {
		return nil, err
	}
	m.sys.Set(st)
	wrapper := m.sys.Tun.Get()
	wrapper.PreFilterPacketInboundFromWireGuard = clamped(wrapper.PreFilterPacketInboundFromWireGuard)
	wrapper.PostFilterPacketOutboundToWireGuard = clamped(wrapper.PostFilterPacketOutboundToWireGuard)
	envknob.Setenv("TS_NETSTACK_KEEPALIVE_IDLE", "15s")
	envknob.Setenv("TS_NETSTACK_KEEPALIVE_INTERVAL", "5s")
	if m.stack, err = netstack.Create(logf, wrapper, m.engine, m.sys.MagicSock.Get(), m.dialer, m.sys.DNSManager.Get(), m.sys.ProxyMapper()); err != nil {
		return nil, err
	}
	m.sys.Set(m.stack)
	m.stack.ProcessLocalIPs = true
	// Security: netstack would otherwise forward unclaimed inbound flows to host services (UDP to 127.0.0.1).
	m.stack.GetTCPHandlerForFlow = func(src, dst netip.AddrPort) (func(net.Conn), bool) { return nil, true }
	m.stack.GetUDPHandlerForFlow = func(src, dst netip.AddrPort) (func(nettype.ConnPacketConn), bool) { return nil, true }
	wrapper.Start()
	m.lb, err = ipnlocal.NewLocalBackend(logf, logid.PublicID{}, m.sys, controlclient.LocalBackendStartKeyOSNeutral)
	if err != nil {
		return nil, err
	}
	if err = m.stack.Start(m.lb); err != nil {
		return nil, err
	}
	m.lb.SetVarRoot(dir)
	ctx, unwatch := context.WithCancel(context.Background())
	m.unwatch = unwatch
	watcherStarted = true
	go func() {
		defer close(m.watched)
		removed := false
		m.lb.WatchNotifications(ctx, ipn.NotifyInitialState|ipn.NotifyInitialNetMap|ipn.NotifyPeerChanges, nil, func(n *ipn.Notify) bool {
			if n.ErrMessage != nil {
				select {
				case m.errors <- *n.ErrMessage:
				default:
				}
			}
			// A machine removed while offline hears it only as a refused registration.
			if removed = n.State != nil && *n.State == ipn.NeedsMachineAuth || n.ErrMessage != nil && *n.ErrMessage == refusedKeys; removed {
				return false
			}
			m.updateGateway()
			return true
		})
		if removed {
			if err := os.WriteFile(filepath.Join(dir, Removed), nil, 0600); err != nil {
				log.Printf("mesh: removal marker: %v", err)
			}
			unwatch()
			m.release()
			close(m.removed)
		}
	}()
	prefs := ipn.NewPrefs()
	prefs.ControlURL, prefs.Hostname, prefs.WantRunning = shim.URL(), name, true
	prefs.CorpDNS = false
	prefs.RouteAll = false
	prefs.AutoUpdate.Check = false
	if err = m.lb.Start(ipn.Options{UpdatePrefs: prefs, AuthKey: secret}); err != nil {
		return nil, err
	}
	if m.lb.State() == ipn.NeedsLogin {
		if err = m.lb.StartLoginInteractive(ctx); err != nil {
			return nil, err
		}
	}
	return m, nil
}

func (m *Mesh) Removed() <-chan struct{} { return m.removed }

// Errors carries backend errors such as a registration the hub refused, which
// the backend keeps retrying. Unread errors past the first are dropped.
func (m *Mesh) Errors() <-chan string { return m.errors }
func (m *Mesh) Self() netip.Addr {
	if nm := m.lb.NetMapWithPeers(); nm != nil && nm.GetAddresses().Len() > 0 {
		return nm.GetAddresses().At(0).Addr()
	}
	return netip.Addr{}
}
func (m *Mesh) Peer(name string) (netip.Addr, bool) {
	if nm := m.lb.NetMapWithPeers(); nm != nil {
		for _, p := range nm.Peers {
			if p.Name() == name && p.Addresses().Len() > 0 {
				return p.Addresses().At(0).Addr(), true
			}
		}
	}
	return netip.Addr{}, false
}

// Logout asks the hub to delete this machine's record. Close still releases the mesh.
func (m *Mesh) Logout(ctx context.Context) error {
	// The watcher stops first: it releases the backend when the hub's answer reads as a removal.
	m.unwatch()
	<-m.watched
	select {
	case <-m.removed:
		return nil // the hub removed the machine first
	default:
	}
	if err := m.lb.Logout(ctx, ipnauth.Self); err != nil && err.Error() != refusedKeys {
		return err
	}
	return nil // a refusal means the hub removed the machine before this logout reached it
}
func (m *Mesh) Close() error {
	m.unwatch()
	<-m.watched
	m.release()
	return nil
}
func (m *Mesh) release() {
	m.resources.Do(func() {
		m.gateway.Store(0)
		close(m.gatewayChanges)
		close(m.done)
		if m.stack != nil {
			m.stack.Close()
		}
		if m.lb != nil {
			m.lb.Shutdown()
		} else if m.engine != nil {
			m.engine.Close()
		}
		if m.monitor != nil {
			m.monitor.Close()
		}
		if m.dialer != nil {
			m.dialer.Close()
		}
		m.sys.Bus.Get().Close()
		m.shim.Close()
	})
}
