//go:build linux || darwin

package supervisor

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"os/user"
	"path/filepath"
	"runtime"
	"strconv"
	"syscall"
)

// Operator is resolved locally; zero values cannot launch a process.
type Operator struct {
	name, home string
	uid, gid   uint32
	groups     []uint32
}

func LookupOperator(name string) (Operator, error) {
	u, err := user.Lookup(name)
	if err != nil {
		return Operator{}, err
	}
	uid, err := strconv.ParseUint(u.Uid, 10, 32)
	if err != nil {
		return Operator{}, err
	}
	gid, err := strconv.ParseUint(u.Gid, 10, 32)
	if err != nil {
		return Operator{}, err
	}
	ids, err := u.GroupIds()
	if err != nil {
		return Operator{}, err
	}
	op := Operator{name: u.Username, home: u.HomeDir, uid: uint32(uid), gid: uint32(gid)}
	for _, id := range ids {
		n, err := strconv.ParseUint(id, 10, 32)
		if err != nil {
			return Operator{}, err
		}
		op.groups = append(op.groups, uint32(n))
	}
	return op, validateOperator(op)
}
func DefaultCacheDir() string {
	if runtime.GOOS == "darwin" {
		return "/Library/Caches/iii-mesh/workers"
	}
	return "/var/cache/iii-mesh/workers"
}
func validateOperator(op Operator) error {
	if op.name == "" || op.uid == 0 || !filepath.IsAbs(op.home) {
		return errors.New("workers require a non-root operator with an absolute home directory")
	}
	if os.Geteuid() != 0 && (uint32(os.Geteuid()) != op.uid || uint32(os.Getegid()) != op.gid) {
		return errors.New("only root may switch to another operator")
	}
	return nil
}
func runAs(cmd *exec.Cmd, op Operator) error {
	if err := validateOperator(op); err != nil {
		return err
	}
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true, Credential: &syscall.Credential{Uid: op.uid, Gid: op.gid, Groups: append([]uint32{}, op.groups...)}}
	if os.Geteuid() != 0 {
		cmd.SysProcAttr.Credential.NoSetGroups = true
		// An unprivileged process cannot set supplementary groups. Refuse a
		// different group set instead of silently retaining unrelated privileges.
		groups, err := os.Getgroups()
		if err != nil {
			return err
		}
		allowed := map[uint32]bool{op.gid: true}
		for _, g := range op.groups {
			allowed[g] = true
		}
		for _, g := range groups {
			if !allowed[uint32(g)] {
				return errors.New("process has groups outside operator credentials")
			}
		}
	}
	return nil
}
func trustedOwner(info os.FileInfo) error {
	st, ok := info.Sys().(*syscall.Stat_t)
	if !ok || (st.Uid != 0 && st.Uid != uint32(os.Geteuid())) {
		return errors.New("supervisor path is not owned by the service or root")
	}
	return nil
}
func checkDirectory(path string, info os.FileInfo, op *Operator) error {
	if !info.IsDir() {
		return fmt.Errorf("not a supervisor directory: %s", path)
	}
	if err := trustedOwner(info); err != nil {
		return fmt.Errorf("%s: %w", path, err)
	}
	st := info.Sys().(*syscall.Stat_t)
	stickyRoot := st.Uid == 0 && info.Mode()&os.ModeSticky != 0
	if info.Mode().Perm()&0022 != 0 && !stickyRoot {
		return fmt.Errorf("supervisor directory is writable by another account: %s", path)
	}
	if op != nil && !operatorCanTraverse(info, *op) {
		return fmt.Errorf("operator cannot traverse worker cache: %s", path)
	}
	return nil
}
func operatorCanTraverse(info os.FileInfo, op Operator) bool {
	st := info.Sys().(*syscall.Stat_t)
	bits := info.Mode().Perm()
	if st.Uid == op.uid {
		return bits&0100 != 0
	}
	group := st.Gid == op.gid
	for _, g := range op.groups {
		group = group || st.Gid == g
	}
	if group {
		return bits&0010 != 0
	}
	return bits&0001 != 0
}
func terminateGroup(cmd *exec.Cmd) { _ = syscall.Kill(-cmd.Process.Pid, syscall.SIGTERM) }
func killGroup(cmd *exec.Cmd)      { _ = syscall.Kill(-cmd.Process.Pid, syscall.SIGKILL) }
