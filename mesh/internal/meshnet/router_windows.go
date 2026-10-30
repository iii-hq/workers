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
	"errors"
	"net/netip"
	"path/filepath"

	"github.com/iii-hq/workers/mesh/internal/service"
	"github.com/tailscale/wireguard-go/tun"
	"golang.org/x/sys/windows"
	"golang.org/x/sys/windows/registry"
	"golang.zx2c4.com/wireguard/windows/tunnel/winipcfg"
	"tailscale.com/net/tstun"
)

const (
	// The wintun adapter's name. Tailscale's is Tailscale.
	tunName = "iii"
	// Windows Defender Firewall blocks what peers open to this machine: the
	// rule of this name lets in the project's /48 to this node's address.
	// Windows takes a packet only on the interface that has its address.
	rule = "iii-mesh"
	// Windows' own metrics run 5 to 75, and a new adapter gets 5, the
	// lowest: multicast and broadcast would leave by the mesh, and so would
	// 169.254/16, since an adapter with no IPv4 address takes a link-local
	// one and its route, and Windows lets no one turn that off at run time.
	// Only the /48 has to leave by the mesh, and no other adapter routes it.
	metric = 1000
)

// tstun asks wintun for Tailscale's adapter GUID, which would take the
// user's Tailscale adapter. One of our own that never changes keeps Windows
// to one network profile for the mesh.
var guid = windows.GUID{Data1: 0x33e50ce1, Data2: 0x72bd, Data3: 0x4bb4, Data4: [8]byte{0xb1, 0x7f, 0x5d, 0x90, 0x87, 0xc9, 0x31, 0x66}}

func init() {
	tun.WintunTunnelType = tunName
	tun.WintunStaticRequestedGUID = &guid
}

// alone has nothing to check: a second worker's adapter of the same GUID
// fails as one that exists.
func alone() error { return service.VerifyWintunInstalled() }

// Up has nothing to do: wintun's adapter is up once made.
func (r *osRouter) Up() error { return nil }

// add gives the adapter address a, routes a's /48 to it, and lets peers in.
// The address and route go with the adapter when a worker is killed; the
// firewall rule stays until the next start or uninstall.
func add(_ string, a netip.Prefix) error {
	luid, err := winipcfg.LUIDFromGUID(&guid)
	if err != nil {
		return err
	}
	return configureWindowsRoute(a, windowsRouteOperations{
		firewall: func() error {
			if err := dropRule(); err != nil {
				return err
			}
			return firewallCommand("New-NetFirewallRule -Name '" + rule + "' -DisplayName 'iii mesh' -Direction Inbound -Action Allow -InterfaceAlias 'iii' -Protocol Any -LocalAddress '" + a.Addr().String() + "' -RemoteAddress '" + project(a).String() + "' -ErrorAction Stop | Out-Null")
		},
		configure: func() error {
			v4, err := luid.IPInterface(windows.AF_INET)
			if err != nil {
				return err
			}
			v4.UseAutomaticMetric, v4.Metric = false, metric
			if err = v4.Set(); err != nil {
				return err
			}
			v6, err := luid.IPInterface(windows.AF_INET6)
			if err != nil {
				return err
			}
			v6.DadTransmits, v6.RouterDiscoveryBehavior, v6.NLMTU = 0, winipcfg.RouterDiscoveryDisabled, uint32(tstun.DefaultTUNMTU())
			v6.UseAutomaticMetric, v6.Metric = false, metric
			if err = v6.Set(); err != nil {
				return err
			}
			for _, stack := range []string{"Tcpip", "Tcpip6"} {
				k, err := registry.OpenKey(registry.LOCAL_MACHINE, `SYSTEM\CurrentControlSet\Services\`+stack+`\Parameters\Interfaces\`+guid.String(), registry.SET_VALUE)
				if err != nil {
					return err
				}
				err = k.SetDWordValue("RegistrationEnabled", 0)
				k.Close()
				if err != nil {
					return err
				}
			}
			return nil
		},
		address: func() error { return luid.AddIPAddress(a) },
		route:   func() error { return luid.AddRoute(project(a), netip.IPv6Unspecified(), 0) },
		cleanup: func() error { return del("", a) },
	})
}

// keep checks nothing: no service here removes what add made, as
// systemd-networkd does on Linux.
func keep(string, netip.Prefix) error { return nil }

func firewallCommand(script string) error {
	directory, err := windows.GetSystemDirectory()
	if err != nil {
		return err
	}
	return run(filepath.Join(directory, "WindowsPowerShell", "v1.0", "powershell.exe"), "-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference='Stop'; "+script)
}

// A scoped firewall rule outlives an adapter after an unexpected service exit.
func dropRule() error {
	return firewallCommand("Get-NetFirewallRule -ErrorAction Stop | Where-Object Name -EQ '" + rule + "' | Remove-NetFirewallRule -ErrorAction Stop")
}
func ignoreAbsent(err error) error {
	if errors.Is(err, windows.ERROR_NOT_FOUND) || errors.Is(err, windows.ERROR_FILE_NOT_FOUND) {
		return nil
	}
	return err
}
func del(_ string, a netip.Prefix) error {
	firewallErr := dropRule()
	luid, err := winipcfg.LUIDFromGUID(&guid)
	if err != nil {
		return errors.Join(firewallErr, ignoreAbsent(err))
	}
	return errors.Join(firewallErr, ignoreAbsent(luid.DeleteRoute(project(a), netip.IPv6Unspecified())), ignoreAbsent(luid.DeleteIPAddress(a)))
}

func platformHostsPath() string {
	directory, err := windows.GetSystemDirectory()
	if err != nil {
		return ""
	}
	return filepath.Join(directory, "drivers", "etc", "hosts")
}
