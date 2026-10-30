// MIT License
//
// Copyright (c) 2026 Alexander Yue
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

package elevate

import (
	"errors"
	"log"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"syscall"
	"unsafe"

	"golang.org/x/sys/windows"
)

const (
	// tokenElevation is TOKEN_INFORMATION_CLASS TokenElevation, which the
	// syscall package does not name.
	tokenElevation = 20
	// console, then the asking process's id, lead the elevated child's args.
	// UAC starts it with an environment of its own, so args are the one way
	// to tell it.
	console = "--console-of"
	// ShellExecuteEx: keep the process handle, and return only once started.
	seeMaskNoCloseProcess = 0x40
	seeMaskNoAsync        = 0x100
	swHide                = 0
)

// x/sys/windows wraps neither ShellExecuteEx nor the console calls.
var (
	shellExecuteEx = windows.NewLazySystemDLL("shell32.dll").NewProc("ShellExecuteExW")
	attachConsole  = windows.NewLazySystemDLL("kernel32.dll").NewProc("AttachConsole")
	freeConsole    = windows.NewLazySystemDLL("kernel32.dll").NewProc("FreeConsole")
)

// stale keeps the os.Stdout and os.Stderr that Arguments replaces reachable.
// FreeConsole closed their handles, and new handles reuse the numbers: the
// finalizer of either would close one of those, the runtime's or the
// console's, and a console child would then fail with 0xc0000142.
var stale []*os.File

// shellExecuteInfo is SHELLEXECUTEINFOW, whose layout Go's alignment matches.
type shellExecuteInfo struct {
	size       uint32
	mask       uint32
	hwnd       windows.Handle
	verb       *uint16
	file       *uint16
	parameters *uint16
	directory  *uint16
	show       int32
	instApp    windows.Handle
	idList     uintptr
	class      *uint16
	keyClass   windows.Handle
	hotKey     uint32
	icon       windows.Handle
	process    windows.Handle
}

var originalOperator string

// Arguments authenticates the still-running caller before trusting its SID.
func Arguments(args []string) ([]string, error) {
	if len(args) == 0 || args[0] != console {
		return args, nil
	}
	if len(args) < 3 || !Is() {
		return nil, errors.New("invalid elevation handoff")
	}
	pid, err := strconv.ParseUint(args[1], 10, 32)
	if err != nil {
		return nil, err
	}
	p, err := windows.OpenProcess(windows.PROCESS_QUERY_LIMITED_INFORMATION, false, uint32(pid))
	if err != nil {
		return nil, err
	}
	defer windows.CloseHandle(p)
	var token windows.Token
	if err = windows.OpenProcessToken(p, windows.TOKEN_QUERY, &token); err != nil {
		return nil, err
	}
	defer token.Close()
	u, err := token.GetTokenUser()
	if err != nil {
		return nil, err
	}
	var parentSession, currentSession uint32
	if err = windows.ProcessIdToSessionId(uint32(pid), &parentSession); err != nil {
		return nil, err
	}
	if err = windows.ProcessIdToSessionId(uint32(os.Getpid()), &currentSession); err != nil {
		return nil, err
	}
	buf := make([]uint16, 32768)
	size := uint32(len(buf))
	if err = windows.QueryFullProcessImageName(p, 0, &buf[0], &size); err != nil {
		return nil, err
	}
	exe, err := os.Executable()
	if err != nil {
		return nil, err
	}
	same := strings.EqualFold(filepath.Clean(exe), filepath.Clean(windows.UTF16ToString(buf[:size])))
	if err = validateWindowsHandoff(u.User.Sid.String(), parentSession, currentSession, same); err != nil {
		return nil, err
	}
	originalOperator = u.User.Sid.String()
	_, _, _ = freeConsole.Call()
	_, _, _ = attachConsole.Call(uintptr(pid))
	out, err := os.OpenFile("CONOUT$", os.O_RDWR, 0)
	if err == nil {
		_ = windows.SetStdHandle(windows.STD_OUTPUT_HANDLE, windows.Handle(out.Fd()))
		_ = windows.SetStdHandle(windows.STD_ERROR_HANDLE, windows.Handle(out.Fd()))
		stale = []*os.File{os.Stdout, os.Stderr}
		os.Stdout, os.Stderr = out, out
		log.SetOutput(out)
	}
	return args[2:], nil
}
func Operator() (string, error) {
	if originalOperator != "" {
		return originalOperator, nil
	}
	u, err := windows.GetCurrentProcessToken().GetTokenUser()
	if err != nil {
		return "", err
	}
	var session uint32
	if err = windows.ProcessIdToSessionId(uint32(os.Getpid()), &session); err != nil {
		return "", err
	}
	sid := u.User.Sid.String()
	if err = validateWindowsHandoff(sid, session, session, true); err != nil {
		return "", err
	}
	return sid, nil
}

// Is says whether this process runs elevated.
func Is() bool {
	t, err := syscall.OpenCurrentProcessToken()
	if err != nil {
		return false
	}
	defer func() { _ = t.Close() }()
	var e, n uint32
	return syscall.GetTokenInformation(t, tokenElevation, (*byte)(unsafe.Pointer(&e)), 4, &n) == nil && e != 0
}

// Run runs this binary with args elevated, which shows the UAC prompt, and
// returns its exit code. Its output shows in this console. A prompt the user
// declines is 1, and returns, so the caller's cleanup runs.
func Run(args ...string) int {
	exe, err := os.Executable()
	if err != nil {
		log.Print(err)
		return 1
	}
	line := console + " " + strconv.Itoa(os.Getpid())
	for _, a := range args {
		line += " " + syscall.EscapeArg(a)
	}
	info := shellExecuteInfo{
		mask:       seeMaskNoCloseProcess | seeMaskNoAsync,
		verb:       windows.StringToUTF16Ptr("runas"),
		file:       windows.StringToUTF16Ptr(exe),
		parameters: windows.StringToUTF16Ptr(line),
		show:       swHide,
	}
	info.size = uint32(unsafe.Sizeof(info))
	if ok, _, err := shellExecuteEx.Call(uintptr(unsafe.Pointer(&info))); ok == 0 {
		log.Print(err)
		return 1
	}
	defer func() { _ = windows.CloseHandle(info.process) }()
	var code uint32
	if _, err := windows.WaitForSingleObject(info.process, windows.INFINITE); err != nil {
		log.Print(err)
		return 1
	}
	if err := windows.GetExitCodeProcess(info.process, &code); err != nil {
		log.Print(err)
		return 1
	}
	return int(code)
}
