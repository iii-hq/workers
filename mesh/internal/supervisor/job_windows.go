package supervisor

import (
	"errors"
	"os/exec"
	"sync"
	"unsafe"

	"golang.org/x/sys/windows"
)

var processJobs sync.Map

func startOperator(cmd *exec.Cmd, op Operator) error {
	return withPreparedCommand(func() (func(), error) { return PrepareCommand(cmd, op) }, func() error {
		job, err := windows.CreateJobObject(nil, nil)
		if err != nil {
			return err
		}
		limits := windows.JOBOBJECT_EXTENDED_LIMIT_INFORMATION{}
		limits.BasicLimitInformation.LimitFlags = windows.JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
		if _, err = windows.SetInformationJobObject(job, windows.JobObjectExtendedLimitInformation, uintptr(unsafe.Pointer(&limits)), uint32(unsafe.Sizeof(limits))); err != nil {
			windows.CloseHandle(job)
			return err
		}
		// A suspended primary thread cannot create descendants before assignment.
		cmd.SysProcAttr.CreationFlags |= windows.CREATE_SUSPENDED
		if err = cmd.Start(); err != nil {
			windows.CloseHandle(job)
			return err
		}
		process, err := windows.OpenProcess(windows.PROCESS_SET_QUOTA|windows.PROCESS_TERMINATE, false, uint32(cmd.Process.Pid))
		if err == nil {
			err = windows.AssignProcessToJobObject(job, process)
			windows.CloseHandle(process)
		}
		if err == nil {
			err = resumePrimaryThread(uint32(cmd.Process.Pid))
		}
		if err != nil {
			cmd.Process.Kill()
			windows.CloseHandle(job)
			cmd.Wait()
			return err
		}
		processJobs.Store(cmd, job)
		return nil
	})
}
func resumePrimaryThread(pid uint32) error {
	snapshot, err := windows.CreateToolhelp32Snapshot(windows.TH32CS_SNAPTHREAD, 0)
	if err != nil {
		return err
	}
	defer windows.CloseHandle(snapshot)
	entry := windows.ThreadEntry32{Size: uint32(unsafe.Sizeof(windows.ThreadEntry32{}))}
	for err = windows.Thread32First(snapshot, &entry); err == nil; err = windows.Thread32Next(snapshot, &entry) {
		if entry.OwnerProcessID != pid {
			continue
		}
		thread, err := windows.OpenThread(windows.THREAD_SUSPEND_RESUME, false, entry.ThreadID)
		if err != nil {
			return err
		}
		defer windows.CloseHandle(thread)
		_, err = windows.ResumeThread(thread)
		return err
	}
	return errors.New("operator process has no suspended primary thread")
}
func terminateGroup(cmd *exec.Cmd) { killGroup(cmd) }
func killGroup(cmd *exec.Cmd) {
	if h, ok := processJobs.LoadAndDelete(cmd); ok {
		windows.CloseHandle(h.(windows.Handle))
	}
}
