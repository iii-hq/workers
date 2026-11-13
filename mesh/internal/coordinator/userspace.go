// Copyright (c) 2020 Tailscale Inc & contributors.
// SPDX-License-Identifier: BSD-3-Clause
// Adapted from tailscale.com/tsnet; see LICENSE.

package coordinator

import (
	"context"
	"errors"
	"fmt"
	"net"
	"net/netip"
	"path/filepath"
	"sync"

	"tailscale.com/control/controlclient"
	"tailscale.com/envknob"
	"tailscale.com/ipn"
	"tailscale.com/ipn/ipnlocal"
	"tailscale.com/net/netmon"
	"tailscale.com/net/tsdial"
	"tailscale.com/tsd"
	"tailscale.com/types/logger"
	"tailscale.com/types/logid"
	"tailscale.com/types/nettype"
	"tailscale.com/util/eventbus"
	"tailscale.com/wgengine"
	"tailscale.com/wgengine/netstack"
)

type userspace struct {
	once      sync.Once
	mu        sync.Mutex
	closed    bool
	address   netip.Addr
	sys       *tsd.System
	engine    wgengine.Engine
	backend   *ipnlocal.LocalBackend
	stack     *netstack.Impl
	monitor   *netmon.Monitor
	dialer    *tsdial.Dialer
	listeners []net.Listener
}

func (u *userspace) close() {
	u.once.Do(func() {
		u.mu.Lock()
		u.closed = true
		listeners := u.listeners
		u.listeners = nil
		u.mu.Unlock()
		for _, l := range listeners {
			_ = l.Close()
		}
		if u.stack != nil {
			_ = u.stack.Close()
		}
		if u.backend != nil {
			u.backend.Shutdown()
		} else if u.engine != nil {
			u.engine.Close()
		}
		if u.monitor != nil {
			_ = u.monitor.Close()
		}
		if u.dialer != nil {
			_ = u.dialer.Close()
		}
		if u.sys != nil {
			u.sys.Bus.Get().Close()
		}
	})
}

// Listen owns an unprivileged engine and its listeners until ctx ends or Close
// is called. Backend state writes share the coordinator's atomic state file.
func (c *Coordinator) Listen(ctx context.Context, control string, ports ...int) (listeners []net.Listener, err error) {
	c.life.Lock()
	defer c.life.Unlock()
	if c.backend != nil {
		return nil, errors.New("hub userspace node already started")
	}
	if c.ctx.Err() != nil {
		return nil, c.ctx.Err()
	}
	for _, p := range ports {
		if p < 0 || p > 65535 {
			return nil, fmt.Errorf("invalid TCP port %d", p)
		}
	}
	ctx, cancel := context.WithCancel(ctx)
	stop := context.AfterFunc(c.ctx, cancel)
	defer func() {
		if err != nil {
			stop()
			cancel()
		}
	}()
	envknob.Setenv("TS_NO_LOGS_NO_SUPPORT", "true")
	// ipnlocal computes a sockstat log directory even when logging is disabled.
	envknob.Setenv("TS_LOGS_DIR", filepath.Dir(c.file))
	envknob.Setenv("TS_NETSTACK_KEEPALIVE_IDLE", "15s")
	envknob.Setenv("TS_NETSTACK_KEEPALIVE_INTERVAL", "5s")
	u := &userspace{sys: tsd.NewSystem()}
	defer func() {
		if err != nil {
			u.close()
		}
	}()
	logf := logger.Discard
	u.monitor, err = netmon.New(u.sys.Bus.Get(), logf)
	if err != nil {
		return nil, err
	}
	u.dialer = &tsdial.Dialer{Logf: logf}
	u.dialer.SetBus(u.sys.Bus.Get())
	u.engine, err = wgengine.NewUserspaceEngine(logf, wgengine.Config{
		ListenPort: 41643, EventBus: u.sys.Bus.Get(), NetMon: u.monitor, Dialer: u.dialer, SetSubsystem: u.sys.Set,
		ControlKnobs: u.sys.ControlKnobs(), HealthTracker: u.sys.HealthTracker.Get(), Metrics: u.sys.UserMetricsRegistry(),
	})
	if err != nil {
		return nil, err
	}
	u.sys.Set(u.engine)
	// A timer does not count the time asleep: the hub renews what it holds by wall time when the machine wakes.
	eventbus.SubscribeFunc(u.sys.Bus.Get().Client("mesh-hub"), func(d netmon.ChangeDelta) {
		if f := c.woke.Load(); d.TimeJumped() && f != nil {
			(*f)()
		}
	})
	u.sys.HealthTracker.Get().SetMetricsRegistry(u.sys.UserMetricsRegistry())
	u.stack, err = netstack.Create(logf, u.sys.Tun.Get(), u.engine, u.sys.MagicSock.Get(), u.dialer, u.sys.DNSManager.Get(), u.sys.ProxyMapper())
	if err != nil {
		return nil, err
	}
	u.sys.Tun.Get().Start()
	u.sys.Set(u.stack)
	u.stack.ProcessLocalIPs = true
	// Unclaimed flows must never fall through to host TCP or UDP services.
	u.stack.GetTCPHandlerForFlow = func(src, dst netip.AddrPort) (func(net.Conn), bool) { return nil, true }
	u.stack.GetUDPHandlerForFlow = func(src, dst netip.AddrPort) (func(nettype.ConnPacketConn), bool) { return nil, true }
	u.sys.Set(c.StateStore())
	u.backend, err = ipnlocal.NewLocalBackend(logf, logid.PublicID{}, u.sys, controlclient.LocalBackendStartKeyOSNeutral)
	if err != nil {
		return nil, err
	}
	// Security: the backend lets peer API connections past the packet filter, and to it every mesh machine is the owner.
	u.sys.Tun.Get().PeerAPIPort = nil
	if err = u.stack.Start(u.backend); err != nil {
		return nil, err
	}
	prefs := ipn.NewPrefs()
	prefs.Hostname = "hub"
	prefs.WantRunning = true
	prefs.ControlURL = control
	if err = u.backend.Start(ipn.Options{UpdatePrefs: prefs, AuthKey: c.self}); err != nil {
		return nil, err
	}
	if u.backend.State() == ipn.NeedsLogin {
		if err = u.backend.StartLoginInteractive(ctx); err != nil {
			return nil, err
		}
	}
	var address netip.Addr
	u.backend.WatchNotifications(ctx, ipn.NotifyInitialState, nil, func(n *ipn.Notify) bool {
		if n.ErrMessage != nil {
			err = errors.New(*n.ErrMessage)
			return false
		}
		if n.State != nil && *n.State == ipn.Running {
			for _, a := range u.backend.StatusWithoutPeers().TailscaleIPs {
				if a.Is6() {
					address = a
					return false
				}
			}
		}
		return true
	})
	if err != nil {
		return nil, err
	}
	if !address.IsValid() {
		return nil, fmt.Errorf("waiting for hub address: %w", ctx.Err())
	}
	for _, p := range ports {
		var l net.Listener
		l, err = u.stack.ListenTCP("tcp6", netip.AddrPortFrom(address, uint16(p)).String())
		if err != nil {
			return nil, err
		}
		u.mu.Lock()
		listeners = append(listeners, u.ownListenerLocked(l))
		u.mu.Unlock()
	}
	u.address = address
	c.backend = u
	go func() { <-ctx.Done(); u.close(); stop(); cancel() }()
	return listeners, nil
}

// OnWake sets f to run when the hub's machine wakes from sleep; f must not block.
func (c *Coordinator) OnWake(f func()) { c.woke.Store(&f) }

// ListenTCP binds another port on the existing hub backend. The caller owns
// closing the returned listener; Close also releases it during shutdown.
func (c *Coordinator) ListenTCP(port int) (net.Listener, error) {
	c.life.Lock()
	defer c.life.Unlock()
	if c.backend == nil || c.ctx.Err() != nil {
		return nil, errors.New("hub backend is not running")
	}
	if port < 1 || port > 65535 {
		return nil, fmt.Errorf("invalid TCP port %d", port)
	}
	u := c.backend
	u.mu.Lock()
	defer u.mu.Unlock()
	if u.closed {
		return nil, errors.New("hub backend is closed")
	}
	l, err := u.stack.ListenTCP("tcp6", netip.AddrPortFrom(u.address, uint16(port)).String())
	if err != nil {
		return nil, err
	}
	return u.ownListenerLocked(l), nil
}

type ownedListener struct {
	net.Listener
	owner *userspace
	once  sync.Once
	err   error
}

func (u *userspace) ownListenerLocked(l net.Listener) net.Listener {
	owned := &ownedListener{Listener: l, owner: u}
	u.listeners = append(u.listeners, owned)
	return owned
}
func (l *ownedListener) Close() error {
	l.once.Do(func() {
		l.err = l.Listener.Close()
		l.owner.mu.Lock()
		defer l.owner.mu.Unlock()
		for i, current := range l.owner.listeners {
			if current == l {
				copy(l.owner.listeners[i:], l.owner.listeners[i+1:])
				l.owner.listeners[len(l.owner.listeners)-1] = nil
				l.owner.listeners = l.owner.listeners[:len(l.owner.listeners)-1]
				break
			}
		}
	})
	return l.err
}
