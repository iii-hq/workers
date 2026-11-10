package supervisor

import (
	"context"
	"errors"
	"fmt"
	"os/exec"
	"time"
)

func workerCommand(path, name string, op Operator, rt Runtime, log *rotatingLog) (*exec.Cmd, error) {
	cmd := exec.Command(path)
	cmd.Dir = op.home
	env := WorkerEnv(rt.Node, rt.Namespace, name)
	env[0] = "III_URL=" + rt.URL
	cmd.Env = append([]string{"PATH=/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin", "HOME=" + op.home, "USER=" + op.name, "LOGNAME=" + op.name}, env...)
	cmd.Stdout = log
	cmd.Stderr = log
	cmd.WaitDelay = 2 * time.Second
	return cmd, nil
}
func (s *Supervisor) monitor(ctx context.Context, w *worker, path string, cmd *exec.Cmd, log *rotatingLog, rt Runtime) {
	defer close(w.done)
	defer log.Close()
	restarts := 0
	for {
		err := waitProcess(ctx, cmd)
		if ctx.Err() != nil {
			return
		}
		if err == nil {
			err = errors.New("worker exited")
		}
		s.update(w, func(st *Status) { st.State = "backoff"; st.Error = err.Error(); st.PID = 0 })
		for {
			timer := time.NewTimer(Backoff(restarts))
			select {
			case <-ctx.Done():
				timer.Stop()
				return
			case <-timer.C:
			}
			restarts++
			// Recheck the digest before every execution, including automatic retries.
			err = verifyArtifact(path, w.desired.SHA256, 0555)
			var binary string
			if err == nil {
				binary, err = executable(path, w.desired.Name)
			}
			if err == nil {
				cmd, err = workerCommand(binary, w.desired.Name, s.operator, rt, log)
			}
			if err == nil {
				err = ctx.Err()
			}
			if err == nil {
				err = startOperator(cmd, s.operator)
			}
			if ctx.Err() != nil {
				if err == nil {
					waitProcess(ctx, cmd)
				}
				return
			}
			s.update(w, func(st *Status) {
				st.Restarts = restarts
				if err == nil {
					st.State = "running"
					st.Error = ""
					st.PID = cmd.Process.Pid
				} else {
					st.State = "backoff"
					st.Error = fmt.Sprintf("restart: %v", err)
					st.PID = 0
				}
			})
			if err == nil {
				break
			}
		}
	}
}
func waitProcess(ctx context.Context, cmd *exec.Cmd) error {
	done := make(chan error, 1)
	go func() { done <- cmd.Wait() }()
	select {
	case err := <-done:
		killGroup(cmd)
		return err
	case <-ctx.Done():
		terminateGroup(cmd)
		timer := time.NewTimer(2 * time.Second)
		defer timer.Stop()
		select {
		case err := <-done:
			killGroup(cmd)
			return err
		case <-timer.C:
			killGroup(cmd)
			return <-done
		}
	}
}
