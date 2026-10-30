package supervisor

import (
	"context"
	"os/exec"
	"sync"
	"time"
)

// StartOperator starts a command as the original operator. On success the caller
// owns cmd.Wait and MUST call cleanup after Wait, or to cancel the entire process
// tree before waiting. Cleanup is idempotent. On failure no resources transfer.
// Tokens and the Windows session parent are released before this call returns;
// the returned cleanup retains the job until command/descendant shutdown.
func StartOperator(cmd *exec.Cmd, op Operator) (cleanup func(), err error) {
	if cmd.WaitDelay == 0 {
		cmd.WaitDelay = 2 * time.Second
	}
	if err := startOperator(cmd, op); err != nil {
		return nil, err
	}
	return sync.OnceFunc(func() { killGroup(cmd) }), nil
}

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
