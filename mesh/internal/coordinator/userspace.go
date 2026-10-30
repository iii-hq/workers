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
	"tailscale.com/wgengine"
	"tailscale.com/wgengine/netstack"
)

type userspace struct {
	once      sync.Once
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
		for _, l := range u.listeners {
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
		u.listeners = append(u.listeners, l)
	}
	c.backend = u
	go func() { <-ctx.Done(); u.close(); stop(); cancel() }()
	return append([]net.Listener(nil), u.listeners...), nil
}
