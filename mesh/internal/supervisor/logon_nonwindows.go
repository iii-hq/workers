//go:build !windows

package supervisor

import "errors"

func CaptureOperatorLogon(Operator) (OperatorLogon, error) {
	return OperatorLogon{}, errors.New("operator logon identities require Windows")
}
func BindOperatorLogon(Operator, OperatorLogon) (Operator, error) {
	return Operator{}, errors.New("operator logon identities require Windows")
}
