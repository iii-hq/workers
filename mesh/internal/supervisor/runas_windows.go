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

package supervisor

import (
	"errors"
	"os"
	"os/exec"
	"os/user"
	"path/filepath"
	"strings"
	"syscall"
	"unsafe"

	"github.com/iii-hq/workers/mesh/internal/winsecurity"
	"golang.org/x/sys/windows"
)

type Operator struct{ name, home, sid string }

func LookupOperator(name string) (Operator, error) {
	var u *user.User
	var err error
	if strings.HasPrefix(name, "S-1-") {
		u, err = user.LookupId(name)
	} else {
		u, err = user.Lookup(name)
	}
	if err != nil {
		return Operator{}, err
	}
	op := Operator{name: u.Username, home: u.HomeDir, sid: u.Uid}
	return op, validateOperator(op)
}
func (op Operator) Name() string { return op.name }
func (op Operator) ID() string   { return op.sid }
func DefaultCacheDir() string {
	root := winsecurity.ProgramData()
	if root == "" {
		return ""
	}
	return filepath.Join(root, "iii-mesh-workers")
}
func validateOperator(op Operator) error {
	sid, err := windows.StringToSid(op.sid)
	if err != nil || op.name == "" || !filepath.IsAbs(op.home) || sid.IsWellKnown(windows.WinLocalSystemSid) || sid.IsWellKnown(windows.WinLocalServiceSid) || sid.IsWellKnown(windows.WinNetworkServiceSid) {
		return errors.New("workers require a local interactive operator identity")
	}
	return nil
}
func operatorToken(op Operator) (windows.Token, error) {
	if err := validateOperator(op); err != nil {
		return 0, err
	}
	var ss *windows.WTS_SESSION_INFO
	var n uint32
	if err := windows.WTSEnumerateSessions(0, 0, 1, &ss, &n); err != nil {
		return 0, err
	}
	defer windows.WTSFreeMemory(uintptr(unsafe.Pointer(ss)))
	var sessions []operatorSession
	tokens := map[uint32]windows.Token{}
	defer func() {
		for _, t := range tokens {
			t.Close()
		}
	}()
	for _, s := range unsafe.Slice(ss, n) {
		if s.SessionID == 0 {
			continue
		}
		var t windows.Token
		if windows.WTSQueryUserToken(s.SessionID, &t) != nil {
			continue
		}
		u, err := t.GetTokenUser()
		if err != nil {
			t.Close()
			continue
		}
		sessions = append(sessions, operatorSession{s.SessionID, u.User.Sid.String(), s.State == windows.WTSActive})
		tokens[s.SessionID] = t
	}
	id, err := selectOperatorSession(op.sid, sessions)
	if err != nil {
		return 0, err
	}
	t := tokens[id]
	delete(tokens, id)
	if t.IsElevated() {
		linked, err := t.GetLinkedToken()
		t.Close()
		if err != nil {
			return 0, err
		}
		t = linked
	}
	u, err := t.GetTokenUser()
	var session, size uint32
	if err == nil {
		err = windows.GetTokenInformation(t, windows.TokenSessionId, (*byte)(unsafe.Pointer(&session)), 4, &size)
	}
	if err != nil || u.User.Sid.String() != op.sid || session != id || t.IsElevated() {
		t.Close()
		return 0, errors.New("operator token identity, session or elevation mismatch")
	}
	return t, nil
}

// PrepareCommand obtains the original operator's unelevated interactive token.
// The caller MUST call cleanup after Start (including failures). The token does
// not outlive process creation; RunOperator additionally owns descendant cleanup.
func PrepareCommand(cmd *exec.Cmd, op Operator) (cleanup func(), err error) {
	if !filepath.IsAbs(cmd.Path) {
		return nil, errors.New("operator command must have an absolute executable")
	}
	t, err := operatorToken(op)
	if err != nil {
		return nil, err
	}
	cleanup = func() { t.Close() }
	env, err := t.Environ(false)
	if err != nil {
		cleanup()
		return nil, err
	}
	home, err := t.GetUserProfileDirectory()
	if err != nil {
		cleanup()
		return nil, err
	}
	// Keep the session environment, then apply explicit non-identity overrides.
	for _, e := range cmd.Env {
		key, _, _ := strings.Cut(e, "=")
		switch strings.ToUpper(key) {
		case "HOME", "USER", "LOGNAME", "PATH":
			continue
		}
		env = append(env, e)
	}
	cmd.Env = append(env, "HOME="+home, "USER="+op.name)
	cmd.Dir = home
	cmd.SysProcAttr = &syscall.SysProcAttr{Token: syscall.Token(t), CreationFlags: windows.CREATE_NO_WINDOW}
	return cleanup, nil
}
func secureDirectory(path string, mode os.FileMode, op *Operator) (string, error) {
	public := op != nil
	if err := winsecurity.EnsureDirectory(path, public); err != nil {
		return "", err
	}
	return path, nil
}
func protectArtifact(f *os.File) error               { return winsecurity.Protect(f.Name(), true) }
func checkArtifact(path string, _ os.FileInfo) error { return winsecurity.Check(path, true) }
func checkLog(path string, _ os.FileInfo) error      { return winsecurity.Check(path, false) }

func createArtifactTemp(dir string) (*os.File, error) {
	return winsecurity.CreatePrivateTemp(dir, ".download-")
}
