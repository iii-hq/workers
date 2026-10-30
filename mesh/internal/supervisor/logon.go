package supervisor

import (
	"errors"
	"strconv"
	"strings"
)

// OperatorLogon identifies one Windows logon, including reuse of a WTS session
// or authentication LUID after logout/reboot. It contains no credentials.
// Decimal strings preserve the 64-bit native identities in JSON journals.
type OperatorLogon struct {
	SID              string `json:"sid"`
	SessionID        uint32 `json:"session_id"`
	AuthenticationID uint64 `json:"authentication_id,string"`
	LogonTime        int64  `json:"logon_time,string"`
}

func (l OperatorLogon) Validate() error {
	parts := strings.Split(l.SID, "-")
	valid := len(parts) >= 4 && len(parts) <= 18 && parts[0] == "S" && parts[1] == "1"
	if valid {
		for i, p := range parts[2:] {
			bits := 32
			if i == 0 {
				bits = 48
			}
			n, err := strconv.ParseUint(p, 10, bits)
			if err != nil || strconv.FormatUint(n, 10) != p {
				valid = false
				break
			}
		}
	}
	if !valid || l.SID == "S-1-5-18" || l.SID == "S-1-5-19" || l.SID == "S-1-5-20" || l.SessionID == 0 || l.SessionID == ^uint32(0) || l.AuthenticationID == 0 || l.LogonTime <= 0 {
		return errors.New("invalid interactive operator logon identity")
	}
	return nil
}
func validateOperatorLogon(saved, actual OperatorLogon, elevated bool) error {
	if err := saved.Validate(); err != nil {
		return err
	}
	if err := actual.Validate(); err != nil {
		return err
	}
	if saved != actual || elevated {
		return errors.New("original operator logon is unavailable or reused")
	}
	return nil
}
