//go:build !windows

package supervisor

import "os/exec"

func prepareCommand(cmd *exec.Cmd, op Operator) (func(), error) {
	if cmd.Env == nil {
		cmd.Env = []string{"PATH=/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin", "HOME=" + op.home, "USER=" + op.name, "LOGNAME=" + op.name}
	}
	if cmd.Dir == "" {
		cmd.Dir = op.home
	}
	return func() {}, runAs(cmd, op)
}
func startOperator(cmd *exec.Cmd, op Operator) error {
	return withPreparedCommand(func() (func(), error) { return prepareCommand(cmd, op) }, cmd.Start)
}
