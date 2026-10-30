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

func TestBoundLogonFailurePreventsParentAndTargetLaunch(t *testing.T) {
	saved := OperatorLogon{SID: "S-1-5-21-1-2-3-1001", SessionID: 2, AuthenticationID: 7, LogonTime: 133000000000000000}
	for _, failure := range []string{"unavailable", "same SID different session", "session reused", "LUID reused"} {
		t.Run(failure, func(t *testing.T) {
			parent, target := false, false
			_, err := startWithSessionParent(&exec.Cmd{}, func() (func(), error) {
				if failure == "unavailable" {
					return nil, errors.New("WTSQueryUserToken: session unavailable")
				}
				actual := saved
				switch failure {
				case "same SID different session":
					actual.SessionID++
				case "session reused":
					actual.AuthenticationID++
				case "LUID reused":
					actual.LogonTime++
				}
				if err := validateOperatorLogon(saved, actual, false); err != nil {
					return nil, err
				}
				return func() {}, nil
			}, func() (func(), func(), error) { parent = true; return func() {}, func() {}, nil }, func(*exec.Cmd) error { target = true; return nil })
			if err == nil || parent || target {
				t.Fatal("mismatched logon reached launch", err, parent, target)
			}
		})
	}
}
