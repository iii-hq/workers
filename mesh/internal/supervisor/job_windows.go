package supervisor

import (
	"os"
	"os/exec"
	"sync"
	"syscall"
	"unsafe"

	"golang.org/x/sys/windows"
)

var processJobs sync.Map

func startOperator(cmd *exec.Cmd, op Operator) error {
	stop, err := startWithSessionParent(cmd,
		func() (func(), error) { return prepareCommand(cmd, op) },
		func() (func(), func(), error) { return createSessionParent(cmd) },
		(*exec.Cmd).Start)
	if err != nil {
		return err
	}
	processJobs.Store(cmd, stop)
	return nil
}

// The anchor is never resumed. Go duplicates only the target's standard I/O
// handles into this process, then inherits them from a parent in the same session.
// The target also inherits this parent's job before any of its threads can run.
func createSessionParent(cmd *exec.Cmd) (release func(), stop func(), err error) {
	job, err := windows.CreateJobObject(nil, nil)
	if err != nil {
		return nil, nil, err
	}
	closeJob := func() { windows.CloseHandle(job) }
	stop = closeJob
	defer func() {
		if err != nil {
			closeJob()
		}
	}()
	limits := windows.JOBOBJECT_EXTENDED_LIMIT_INFORMATION{}
	limits.BasicLimitInformation.LimitFlags = windows.JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
	if _, err = windows.SetInformationJobObject(job, windows.JobObjectExtendedLimitInformation, uintptr(unsafe.Pointer(&limits)), uint32(unsafe.Sizeof(limits))); err != nil {
		return nil, nil, err
	}
	attrs, err := windows.NewProcThreadAttributeList(1)
	if err != nil {
		return nil, nil, err
	}
	defer attrs.Delete()
	// ProcThreadAttributeValue(ProcThreadAttributeJobList=13, false, true, false).
	const jobList = 0x0002000d
	if err = attrs.Update(jobList, unsafe.Pointer(&job), unsafe.Sizeof(job)); err != nil {
		return nil, nil, err
	}
	self, err := os.Executable()
	if err != nil {
		return nil, nil, err
	}
	application, err := windows.UTF16PtrFromString(self)
	if err != nil {
		return nil, nil, err
	}
	directory, err := windows.UTF16PtrFromString(cmd.Dir)
	if err != nil {
		return nil, nil, err
	}
	startup := windows.StartupInfoEx{
		StartupInfo:             windows.StartupInfo{Cb: uint32(unsafe.Sizeof(windows.StartupInfoEx{}))},
		ProcThreadAttributeList: attrs.List(),
	}
	var process windows.ProcessInformation
	// An explicit empty environment prevents service secrets entering the anchor.
	emptyEnvironment := [2]uint16{}
	flags := uint32(windows.CREATE_SUSPENDED | windows.CREATE_NO_WINDOW | windows.CREATE_UNICODE_ENVIRONMENT | windows.EXTENDED_STARTUPINFO_PRESENT)
	err = windows.CreateProcessAsUser(windows.Token(cmd.SysProcAttr.Token), application, nil, nil, nil, false, flags, &emptyEnvironment[0], directory, &startup.StartupInfo, &process)
	if err != nil {
		return nil, nil, err
	}
	windows.CloseHandle(process.Thread)
	cmd.SysProcAttr.ParentProcess = syscall.Handle(process.Process)
	release = func() {
		if windows.TerminateProcess(process.Process, 0) == nil {
			windows.WaitForSingleObject(process.Process, 5000)
		}
		windows.CloseHandle(process.Process)
		cmd.SysProcAttr.ParentProcess = 0
	}
	return release, stop, nil
}

func terminateGroup(cmd *exec.Cmd) { killGroup(cmd) }
func killGroup(cmd *exec.Cmd) {
	if stop, ok := processJobs.LoadAndDelete(cmd); ok {
		stop.(func())()
	}
}
