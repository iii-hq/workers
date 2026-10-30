package meshnet

import (
	"errors"
	"net/netip"
	"reflect"
	"testing"
)

func TestWindowsRouteRollback(t *testing.T) {
	for _, fail := range []string{"", "firewall", "interface", "address", "route"} {
		t.Run(fail, func(t *testing.T) {
			var calls []string
			operation := func(name string) func() error {
				return func() error {
					calls = append(calls, name)
					if name == fail {
						return errors.New(name)
					}
					return nil
				}
			}
			ops := windowsRouteOperations{firewall: operation("firewall"), configure: operation("interface"), address: operation("address"), route: operation("route"), cleanup: operation("cleanup")}
			err := configureWindowsRoute(netip.MustParsePrefix("fd42:abcd:1234::2/128"), ops)
			want := []string{"firewall", "interface", "address", "route"}
			if fail != "" {
				for i, s := range want {
					if s == fail {
						want = append(want[:i+1], "cleanup")
						break
					}
				}
			}
			if (err == nil) != (fail == "") || !reflect.DeepEqual(calls, want) {
				t.Fatal(err, calls, want)
			}
		})
	}
}
