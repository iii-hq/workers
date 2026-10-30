package supervisor

import (
	"context"
	"os/exec"
	"time"
)

// RunOperator runs a command with the same authenticated operator and process
// lifetime as workers. Windows requires that operator's existing logon session.
// Callers provide an absolute executable and explicit environment overrides;
// the service environment is never inherited. Cancellation stops descendants.
func RunOperator(ctx context.Context, cmd *exec.Cmd, op Operator) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	if cmd.WaitDelay == 0 {
		cmd.WaitDelay = 2 * time.Second
	}
	if err := startOperator(cmd, op); err != nil {
		return err
	}
	return waitProcess(ctx, cmd)
}
