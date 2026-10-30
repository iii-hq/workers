package supervisor

import (
	"bytes"
	"errors"
	"os/exec"
	"reflect"
	"testing"
)

func TestSessionParentLaunchOwnership(t *testing.T) {
	boom := errors.New("launch failed")
	for _, failure := range []string{"token", "anchor", "target", ""} {
		t.Run("failure_"+failure, func(t *testing.T) {
			var calls []string
			record := func(s string) { calls = append(calls, s) }
			var input, output, diagnostic bytes.Buffer
			cmd := &exec.Cmd{Path: "/absolute/worker", Stdin: &input, Stdout: &output, Stderr: &diagnostic}
			cleanup, err := startWithSessionParent(cmd,
				func() (func(), error) {
					record("token")
					if failure == "token" {
						return nil, boom
					}
					return func() { record("close token") }, nil
				},
				func() (func(), func(), error) {
					record("anchor")
					if failure == "anchor" {
						return nil, nil, boom
					}
					return func() { record("close anchor") }, func() { record("close job") }, nil
				},
				func(got *exec.Cmd) error {
					record("target")
					if got != cmd || got.Stdin != &input || got.Stdout != &output || got.Stderr != &diagnostic {
						t.Fatal("redirected standard I/O was replaced or discarded")
					}
					if failure == "target" {
						return boom
					}
					return nil
				})
			want := map[string][]string{
				"token":  {"token"},
				"anchor": {"token", "anchor", "close token"},
				"target": {"token", "anchor", "target", "close job", "close anchor", "close token"},
				"":       {"token", "anchor", "target", "close anchor", "close token"},
			}[failure]
			if (err != nil) != (failure != "") || !reflect.DeepEqual(calls, want) {
				t.Fatalf("error=%v calls=%v want=%v", err, calls, want)
			}
			if failure != "" {
				if cleanup != nil {
					t.Fatal("failed launch transferred ownership")
				}
				return
			}
			// A successful start keeps the job until normal wait or cancellation.
			cleanup()
			cleanup()
			if !reflect.DeepEqual(calls, append(want, "close job")) {
				t.Fatal(calls)
			}
		})
	}
}

func TestOperatorTokenBinding(t *testing.T) {
	for _, tc := range []struct {
		sid      string
		session  uint32
		elevated bool
		valid    bool
	}{
		{"saved", 4, false, true},
		{"other", 4, false, false},
		{"saved", 5, false, false},
		{"saved", 0, false, false},
		{"saved", 4, true, false},
	} {
		if err := validateSessionToken("saved", 4, tc.sid, tc.session, tc.elevated); (err == nil) != tc.valid {
			t.Fatalf("%+v: %v", tc, err)
		}
	}
}
