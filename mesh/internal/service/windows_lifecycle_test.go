package service

import (
	"errors"
	"reflect"
	"testing"
)

func TestWindowsInstallLifecycle(t *testing.T) {
	for _, fail := range []string{"", "fetch", "stop", "place", "register"} {
		t.Run(fail, func(t *testing.T) {
			var calls []string
			f := func(name string) func() error {
				return func() error {
					calls = append(calls, name)
					if name == fail {
						return errors.New(name)
					}
					return nil
				}
			}
			err := installWindows(windowsInstallation{fetch: f("fetch"), stop: f("stop"), place: f("place"), register: f("register"), start: f("start")})
			want := []string{"fetch", "stop", "place", "register", "start"}
			if fail != "" {
				for i, s := range want {
					if s == fail {
						want = want[:i+1]
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

func TestWindowsUninstallStopsBeforeDeleting(t *testing.T) {
	for _, fail := range []string{"", "reset", "stop", "delete"} {
		t.Run(fail, func(t *testing.T) {
			var calls []string
			step := func(name string) func() error {
				return func() error {
					calls = append(calls, name)
					if name == fail {
						return errors.New(name)
					}
					return nil
				}
			}
			err := uninstallWindows(step("reset"), step("stop"), step("delete"))
			want := []string{"reset", "stop", "delete"}
			if fail != "" {
				for i, s := range want {
					if s == fail {
						want = want[:i+1]
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
