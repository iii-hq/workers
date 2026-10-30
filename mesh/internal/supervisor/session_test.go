package supervisor

import (
	"errors"
	"testing"
)

func TestOperatorSessionSelection(t *testing.T) {
	for _, tc := range []struct {
		name     string
		sessions []operatorSession
		want     int
	}{
		{"active original", []operatorSession{{1, "other", true}, {2, "operator", false}, {3, "operator", true}}, 3},
		{"disconnected original", []operatorSession{{1, "other", true}, {2, "operator", false}}, 2},
		{"no fallback", []operatorSession{{1, "other", true}, {0, "S-1-5-18", false}}, -1},
		{"never session zero", []operatorSession{{0, "operator", true}}, -1},
	} {
		t.Run(tc.name, func(t *testing.T) {
			got, err := selectOperatorSession("operator", tc.sessions)
			if tc.want < 0 {
				if err == nil {
					t.Fatal("accepted missing operator")
				}
			} else if err != nil || int(got) != tc.want {
				t.Fatal(got, err)
			}
		})
	}
}
func TestStartOperatorClosesCredentialsAndStopsFailedStart(t *testing.T) {
	boom := errors.New("start refused")
	calls := []string{}
	err := withPreparedCommand(func() (func(), error) {
		calls = append(calls, "prepare")
		return func() { calls = append(calls, "close token") }, nil
	}, func() error { calls = append(calls, "start"); return boom })
	if !errors.Is(err, boom) || len(calls) != 3 || calls[2] != "close token" {
		t.Fatal(err, calls)
	}
	calls = nil
	err = withPreparedCommand(func() (func(), error) { return nil, boom }, func() error { calls = append(calls, "start"); return nil })
	if !errors.Is(err, boom) || len(calls) != 0 {
		t.Fatal(err, calls)
	}
}

func TestOperatorLogonIdentity(t *testing.T) {
	saved := OperatorLogon{SID: "S-1-5-21-1-2-3-1001", SessionID: 2, AuthenticationID: 0x100000002, LogonTime: 133000000000000000}
	for _, change := range []string{"same", "other SID", "other session", "reused session", "reused LUID", "elevated"} {
		t.Run(change, func(t *testing.T) {
			actual := saved
			switch change {
			case "other SID":
				actual.SID = "S-1-5-21-1-2-3-1002"
			case "other session":
				actual.SessionID++
			case "reused session":
				actual.AuthenticationID++
			case "reused LUID":
				actual.LogonTime++
			}
			err := validateOperatorLogon(saved, actual, change == "elevated")
			if (err == nil) != (change == "same") {
				t.Fatal(change, err)
			}
		})
	}
	for _, change := range []string{"zero session", "current session", "zero LUID", "zero time", "system", "noncanonical SID", "invalid SID", "overflow SID"} {
		t.Run(change, func(t *testing.T) {
			bad := saved
			switch change {
			case "zero session":
				bad.SessionID = 0
			case "current session":
				bad.SessionID = ^uint32(0)
			case "zero LUID":
				bad.AuthenticationID = 0
			case "zero time":
				bad.LogonTime = 0
			case "system":
				bad.SID = "S-1-5-18"
			case "noncanonical SID":
				bad.SID = "S-1-5-21-01"
			case "invalid SID":
				bad.SID = "operator"
			case "overflow SID":
				bad.SID = "S-1-5-4294967296"
			}
			if bad.Validate() == nil {
				t.Fatal("accepted invalid identity", bad)
			}
		})
	}
}
