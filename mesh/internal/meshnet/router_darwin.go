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
	"fmt"
	"log"
	"net/netip"
	"os/exec"
	"strings"
)

// macOS names every TUN itself: utun and the next free number.
const tunName = "utun"

func (r *osRouter) Up() error { return run("ifconfig", r.tun, "up") }

// add gives tun address a and routes a's /48 to it. Both go with the TUN
// when a worker is killed. route add exits 0 on a route that exists, so the
// route is read back: a second worker finds the first's TUN holding it.
func add(tun string, a netip.Prefix) error {
	if err := run("ifconfig", tun, "inet6", a.String(), a.Addr().String()); err != nil {
		return err
	}
	p := project(a).String()
	if err := run("route", "-q", "-n", "add", "-inet6", p, "-iface", tun); err != nil {
		return err
	}
	h, err := holder(p)
	if err != nil {
		return err
	}
	if h != tun {
		return taken(h, p)
	}
	return nil
}

// The project prefix is learned from the first map and checked by add.
func alone() error { return nil }

func taken(h, p string) error {
	return fmt.Errorf("%s holds the route to %s: another worker runs on this machine", h, p)
}

// keep routes a's /48 to tun again when the route is gone, as when a
// worker from before this fix stopped beside this one and took it.
func keep(tun string, a netip.Prefix) error {
	p := project(a).String()
	h, err := holder(p)
	if err != nil || h == tun {
		return err
	}
	if h != "" {
		return taken(h, p)
	}
	if err := run("route", "-q", "-n", "add", "-inet6", p, "-iface", tun); err != nil {
		return err
	}
	log.Printf("mesh: the route to %s was gone: added again", p)
	return nil
}

// dropRule has nothing to drop: the route goes with the TUN.
func dropRule() {}

// del takes the route only from tun: route delete takes it from whatever
// interface holds it.
func del(tun string, a netip.Prefix) error {
	p := project(a).String()
	h, err := holder(p)
	if err != nil {
		return err
	}
	if h == tun {
		if err := run("route", "-q", "-n", "delete", "-inet6", p, "-iface", tun); err != nil {
			return err
		}
	}
	return run("ifconfig", tun, "inet6", a.String(), "-alias")
}

// holder is the interface the route to p leaves by, "" when there is none.
func holder(p string) (string, error) {
	out, err := exec.Command("netstat", "-rn", "-f", "inet6").Output()
	if err != nil {
		return "", fmt.Errorf("netstat: %w", err)
	}
	for _, l := range strings.Split(string(out), "\n") {
		if f := strings.Fields(l); len(f) >= 4 && f[0] == p {
			return f[3], nil
		}
	}
	return "", nil
}
