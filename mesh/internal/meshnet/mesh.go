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
	"errors"
	"fmt"
	"log"
	"net"
	"net/netip"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/tailscale/wireguard-go/tun"
	"tailscale.com/control/controlclient"
	"tailscale.com/envknob"
	_ "tailscale.com/feature/condregister/portmapper"
	"tailscale.com/health"
	"tailscale.com/ipn"
	"tailscale.com/ipn/ipnauth"
	"tailscale.com/ipn/ipnlocal"
	"tailscale.com/ipn/ipnstate"
	"tailscale.com/ipn/store"
	"tailscale.com/net/netmon"
	"tailscale.com/net/tsdial"
	"tailscale.com/net/tstun"
	"tailscale.com/safesocket"
	"tailscale.com/tailcfg"
	"tailscale.com/tsd"
	"tailscale.com/types/key"
	"tailscale.com/types/logger"
	"tailscale.com/types/logid"
	"tailscale.com/wgengine"
	"tailscale.com/wgengine/router"
)

const DefaultPort uint16 = 41643
const Removed = "removed"

type Mesh struct {
	lb        *ipnlocal.LocalBackend
	sys       *tsd.System
	engine    wgengine.Engine
	monitor   *netmon.Monitor
	dialer    *tsdial.Dialer
	health    *health.Tracker
	shim      *Shim
	pinned    key.MachinePublic
	unwatch   context.CancelFunc
	watched   chan struct{}
	removed   chan struct{}
	left      sync.Once
	resources sync.Once
	err       error
	sock      net.Listener
	router    *osRouter
	hostsFile string
	report    atomic.Pointer[func() string]
}

func Up(dir, publicURL, secret, name string, pinned key.MachinePublic) (*Mesh, error) {
	return UpWithPort(dir, publicURL, secret, name, pinned, DefaultPort)
}

// Port zero lets upstream choose an available UDP port.
func UpWithPort(dir, publicURL, secret, name string, pinned key.MachinePublic, udpPort uint16) (*Mesh, error) {
	if !nodeName.MatchString(name) {
		return nil, fmt.Errorf("invalid node name %q", name)
	}
	if err := alone(); err != nil {
		return nil, err
	}
	shim, err := NewShim(publicURL, pinned)
	if err != nil {
		return nil, err
	}
	dev, tunName, err := tstun.New(logger.Discard, tunName)
	if err != nil {
		shim.Close()
		return nil, err
	}
	rt := &osRouter{tun: tunName}
	m, err := startMesh(dir, shim, secret, name, udpPort, dev, rt, hostsPath())
	if err != nil {
		return nil, err
	}
	m.router = rt
	if m.sock, err = safesocket.Listen(socket); err != nil {
		log.Printf("mesh: status socket: %v", err)
	} else {
		go m.serve()
	}
	return m, nil
}

// Passing a device and router transfers their ownership to the backend. A nil
// device selects upstream's userspace TUN, so local checks need no host changes.
func startMesh(dir string, shim *Shim, secret, name string, udpPort uint16, dev tun.Device, rt router.Router, hostsFile string) (_ *Mesh, err error) {
	m := &Mesh{sys: tsd.NewSystem(), shim: shim, pinned: shim.pinned, watched: make(chan struct{}), removed: make(chan struct{}), hostsFile: hostsFile}
	watcherStarted := false
	defer func() {
		if err != nil {
			if watcherStarted {
				m.unwatch()
				<-m.watched
			}
			m.release()
			if dev != nil && m.engine == nil {
				dev.Close()
			}
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
	ready := make(chan struct{})
	if dev != nil {
		dev = newFragmenter(dev, func(ip netip.Addr) bool {
			<-ready
			if m.engine == nil {
				return false
			}
			pong := make(chan *ipnstate.PingResult, 1)
			m.engine.Ping(ip, tailcfg.PingDisco, probeSize, func(r *ipnstate.PingResult) { pong <- r })
			select {
			case r := <-pong:
				return r.Err == ""
			case <-time.After(probeWait):
				return false
			}
		})
	}
	m.engine, err = wgengine.NewUserspaceEngine(logf, wgengine.Config{Tun: dev, Router: rt, ListenPort: udpPort, EventBus: m.sys.Bus.Get(), NetMon: m.monitor, Dialer: m.dialer, SetSubsystem: m.sys.Set, ControlKnobs: m.sys.ControlKnobs(), HealthTracker: m.sys.HealthTracker.Get(), Metrics: m.sys.UserMetricsRegistry()})
	close(ready)
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
	wrapper.Start()
	m.lb, err = ipnlocal.NewLocalBackend(logf, logid.PublicID{}, m.sys, controlclient.LocalBackendStartKeyOSNeutral)
	if err != nil {
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
			if removed = n.State != nil && *n.State == ipn.NeedsMachineAuth; removed {
				return false
			}
			if err := WriteHostsBlock(m.hostsFile, names(m.lb.NetMapWithPeers())); err != nil {
				log.Printf("mesh: hosts: %v", err)
			}
			return true
		})
		if removed {
			if err := os.WriteFile(filepath.Join(dir, Removed), nil, 0600); err != nil {
				log.Printf("mesh: removal marker: %v", err)
			}
			unwatch()
			if err := m.leave(); err != nil {
				log.Printf("mesh: leaving: %v", err)
			}
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

func (m *Mesh) Report(f func() string)   { m.report.Store(&f) }
func (m *Mesh) Removed() <-chan struct{} { return m.removed }
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
func (m *Mesh) Close() error {
	if m.sock != nil {
		m.sock.Close()
	}
	m.unwatch()
	<-m.watched
	return m.leave()
}
func (m *Mesh) Logout() error {
	m.unwatch()
	<-m.watched
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	return m.lb.Logout(ctx, ipnauth.Self)
}
func (m *Mesh) leave() error {
	m.left.Do(func() { m.release(); m.err = WriteHostsBlock(m.hostsFile, nil) })
	return m.err
}
func (m *Mesh) release() {
	m.resources.Do(func() {
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
func (m *Mesh) Reroute() error {
	if m.router == nil {
		return nil
	}
	m.router.mu.Lock()
	defer m.router.mu.Unlock()
	if !m.router.addr.IsValid() {
		return nil
	}
	return keep(m.router.tun, m.router.addr)
}

func Clean() error {
	if err := alone(); err != nil {
		return err
	}
	ruleErr := dropRule()
	var err error
	if runtime.GOOS != "windows" {
		if c, e := net.Dial("unix", socket); e == nil {
			c.Close()
		} else if e := os.Remove(socket); !os.IsNotExist(e) {
			err = e
		}
	}
	return errors.Join(ruleErr, err, WriteHostsBlock(hostsPath(), nil))
}

type osRouter struct {
	mu   sync.Mutex
	tun  string
	addr netip.Prefix
}

func (r *osRouter) Set(c *router.Config) error {
	r.mu.Lock()
	defer r.mu.Unlock()
	var a netip.Prefix
	if c != nil {
		for _, p := range c.LocalAddrs {
			if p.Addr().Is6() {
				a = p
			}
		}
	}
	if a == r.addr {
		return nil
	}
	if r.addr.IsValid() {
		if err := del(r.tun, r.addr); err != nil {
			return err
		}
		r.addr = netip.Prefix{}
	}
	if !a.IsValid() {
		return nil
	}
	if err := add(r.tun, a); err != nil {
		return errors.Join(err, del(r.tun, a))
	}
	r.addr = a
	return nil
}
func (r *osRouter) Close() error          { return r.Set(nil) }
func project(a netip.Prefix) netip.Prefix { return netip.PrefixFrom(a.Addr(), 48).Masked() }
func run(name string, args ...string) error {
	out, err := exec.Command(name, args...).CombinedOutput()
	if err != nil {
		return fmt.Errorf("%s %s: %w: %s", name, strings.Join(args, " "), err, out)
	}
	return nil
}
