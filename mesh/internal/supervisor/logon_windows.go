package supervisor

import (
	"errors"
	"fmt"
	"unsafe"

	"golang.org/x/sys/windows"
)

// CaptureOperatorLogon selects the same authenticated interactive operator as an
// unbound worker launch. Call only before acquiring logon-scoped resources.
func CaptureOperatorLogon(op Operator) (OperatorLogon, error) {
	op.logon = nil
	t, err := operatorToken(op)
	if err != nil {
		return OperatorLogon{}, err
	}
	defer t.Close()
	return tokenLogon(t)
}

// BindOperatorLogon returns an operator that never selects a different logon.
// Each launch queries and revalidates the actual token before creating processes.
func BindOperatorLogon(op Operator, saved OperatorLogon) (Operator, error) {
	if err := validateOperator(op); err != nil {
		return Operator{}, err
	}
	if err := saved.Validate(); err != nil {
		return Operator{}, err
	}
	if saved.SID != op.sid {
		return Operator{}, errors.New("bound logon does not belong to the operator")
	}
	op.logon = &saved
	return op, nil
}

// Native TOKEN_STATISTICS and the prefix of SECURITY_LOGON_SESSION_DATA through
// LogonTime. LSA owns the latter allocation; its Size guards field access.
type tokenStatistics struct {
	TokenID, AuthenticationID                                                                   windows.LUID
	ExpirationTime                                                                              int64
	TokenType, ImpersonationLevel, DynamicCharged, DynamicAvailable, GroupCount, PrivilegeCount uint32
	ModifiedID                                                                                  windows.LUID
}
type logonSessionData struct {
	Size                                         uint32
	LogonID                                      windows.LUID
	UserName, LogonDomain, AuthenticationPackage windows.NTUnicodeString
	LogonType, Session                           uint32
	SID                                          *windows.SID
	LogonTime                                    int64
}

var (
	lsaLibrary             = windows.NewLazySystemDLL("secur32.dll")
	lsaGetLogonSessionData = lsaLibrary.NewProc("LsaGetLogonSessionData")
	lsaFreeReturnBuffer    = lsaLibrary.NewProc("LsaFreeReturnBuffer")
)

func luidValue(l windows.LUID) uint64 { return uint64(uint32(l.HighPart))<<32 | uint64(l.LowPart) }
func tokenLogon(t windows.Token) (OperatorLogon, error) {
	var l OperatorLogon
	u, err := t.GetTokenUser()
	if err != nil {
		return l, err
	}
	l.SID = u.User.Sid.String()
	var size uint32
	if err = windows.GetTokenInformation(t, windows.TokenSessionId, (*byte)(unsafe.Pointer(&l.SessionID)), 4, &size); err != nil {
		return l, err
	}
	var stats tokenStatistics
	if err = windows.GetTokenInformation(t, windows.TokenStatistics, (*byte)(unsafe.Pointer(&stats)), uint32(unsafe.Sizeof(stats)), &size); err != nil {
		return l, err
	}
	l.AuthenticationID = luidValue(stats.AuthenticationID)
	var data *logonSessionData
	status, _, _ := lsaGetLogonSessionData.Call(uintptr(unsafe.Pointer(&stats.AuthenticationID)), uintptr(unsafe.Pointer(&data)))
	if status != 0 {
		return l, fmt.Errorf("LsaGetLogonSessionData: %w", windows.NTStatus(status))
	}
	if data == nil {
		return l, errors.New("LSA returned no operator logon data")
	}
	defer lsaFreeReturnBuffer.Call(uintptr(unsafe.Pointer(data)))
	if data.Size < uint32(unsafe.Offsetof(data.LogonTime)+unsafe.Sizeof(data.LogonTime)) || data.SID == nil {
		return l, errors.New("LSA returned incomplete operator logon data")
	}
	if luidValue(data.LogonID) != l.AuthenticationID || data.Session != l.SessionID || data.SID.String() != l.SID {
		return l, errors.New("LSA logon and operator token identities disagree")
	}
	l.LogonTime = data.LogonTime
	return l, l.Validate()
}
