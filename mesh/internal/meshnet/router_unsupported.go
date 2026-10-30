//go:build !linux && !darwin

package meshnet

import (
	"fmt"
	"net/netip"
	"runtime"
)

const tunName = "iii"

func unsupported() error {
	return fmt.Errorf("mesh TUN routing is not implemented on %s", runtime.GOOS)
}
func alone() error                    { return unsupported() }
func (*osRouter) Up() error           { return unsupported() }
func add(string, netip.Prefix) error  { return unsupported() }
func keep(string, netip.Prefix) error { return unsupported() }
func del(string, netip.Prefix) error  { return unsupported() }
func dropRule()                       {}
