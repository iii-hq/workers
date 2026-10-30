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
