package meshnet

import (
	"errors"
	"net/netip"
)

type windowsRouteOperations struct{ firewall, configure, address, route, cleanup func() error }

func configureWindowsRoute(a netip.Prefix, o windowsRouteOperations) error {
	if !a.IsValid() || !a.Addr().Is6() {
		return errors.New("Windows mesh requires IPv6")
	}
	for _, step := range []func() error{o.firewall, o.configure, o.address, o.route} {
		if err := step(); err != nil {
			return errors.Join(err, o.cleanup())
		}
	}
	return nil
}
