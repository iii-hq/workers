package elevate

import "testing"

func TestWindowsOperatorHandoff(t *testing.T) {
	for _, tc := range []struct {
		name, sid     string
		parent, child uint32
		same          bool
		valid         bool
	}{
		{"original caller", "S-1-5-21-123", 2, 2, true, true},
		{"different session", "S-1-5-21-123", 3, 2, true, false},
		{"system", "S-1-5-18", 0, 0, true, false},
		{"unrelated executable", "S-1-5-21-123", 2, 2, false, false},
		{"service session", "S-1-5-21-123", 0, 0, true, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if err := validateWindowsHandoff(tc.sid, tc.parent, tc.child, tc.same); (err == nil) != tc.valid {
				t.Fatal(err)
			}
		})
	}
}
