//go:build !linux && !darwin

package supervisor

import (
	"errors"
	"os"
	"os/exec"
)

type Operator struct{ name, home string }

var errUnsupported = errors.New("operator worker execution is not supported on this platform")

func LookupOperator(string) (Operator, error)             { return Operator{}, errUnsupported }
func DefaultCacheDir() string                             { return "" }
func validateOperator(Operator) error                     { return errUnsupported }
func runAs(*exec.Cmd, Operator) error                     { return errUnsupported }
func trustedOwner(os.FileInfo) error                      { return errUnsupported }
func checkDirectory(string, os.FileInfo, *Operator) error { return errUnsupported }
func terminateGroup(cmd *exec.Cmd) {
	if cmd.Process != nil {
		_ = cmd.Process.Kill()
	}
}
func killGroup(cmd *exec.Cmd) {
	if cmd.Process != nil {
		_ = cmd.Process.Kill()
	}
}
