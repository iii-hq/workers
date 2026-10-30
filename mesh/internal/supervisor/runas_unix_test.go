//go:build linux || darwin

package supervisor

import (
	"bytes"
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"testing"
	"time"
)

func TestOperatorRefusesRootAndMissingIdentity(t *testing.T) {
	if _, err := LookupOperator("root"); err == nil {
		t.Fatal("root operator accepted")
	}
	if err := runAs(exec.Command("true"), Operator{}); err == nil {
		t.Fatal("zero operator accepted")
	}
	s := testSupervisor(t)
	cmd := exec.Command("true")
	if err := runAs(cmd, s.operator); err != nil {
		t.Fatal(err)
	}
	if c := cmd.SysProcAttr.Credential; c == nil || c.Uid != s.operator.uid || c.Gid != s.operator.gid {
		t.Fatalf("process credential = %+v", c)
	}
	if !cmd.SysProcAttr.Setpgid {
		t.Fatal("children must have a separate process group")
	}
	other := s.operator
	other.uid++
	if os.Geteuid() != 0 {
		if err := runAs(exec.Command("true"), other); err == nil {
			t.Fatal("different operator silently accepted")
		}
	}
}
func TestCachePermissionsAtOperatorBoundary(t *testing.T) {
	s := testSupervisor(t)
	d, _ := artifact(t, s, "#!/bin/sh\nexec sleep 60\n")
	parent := t.TempDir()
	if err := os.Chmod(parent, 0777); err != nil {
		t.Fatal(err)
	}
	s.cacheDir = filepath.Join(parent, "cache")
	rs := s.Apply(context.Background(), []Desired{d})
	if len(rs) != 1 || !strings.Contains(rs[0].Error, "writable") {
		t.Fatalf("writable parent accepted: %+v", rs)
	}
	if _, err := os.Stat(s.cacheDir); !os.IsNotExist(err) {
		t.Fatalf("cache created in untrusted directory: %v", err)
	}
	// Synthetic stat records exercise the dropped credential's access, even
	// when this test runner cannot chown directories or change its UID.
	info, err := os.Stat(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	private := ownedInfo{FileInfo: info, mode: 0700, stat: syscall.Stat_t{Uid: 0, Gid: 0}}
	if operatorCanTraverse(private, s.operator) {
		t.Fatal("root's private state is traversable by operator")
	}
	public := private
	public.mode = 0755
	if !operatorCanTraverse(public, s.operator) {
		t.Fatal("operator cannot traverse root public cache")
	}
}

type ownedInfo struct {
	os.FileInfo
	mode os.FileMode
	stat syscall.Stat_t
}

func (i ownedInfo) Mode() os.FileMode { return os.ModeDir | i.mode }
func (i ownedInfo) Sys() any          { return &i.stat }
func TestPrivilegedCredentialAndPrivateStateBoundary(t *testing.T) {
	if os.Geteuid() != 0 {
		t.Skip("requires root to exercise native credential drop")
	}
	op, err := LookupOperator("nobody")
	if err != nil {
		t.Fatal(err)
	}
	dir, err := os.MkdirTemp("/tmp", "mesh-supervisor-privileged-")
	if err != nil {
		t.Fatal(err)
	}
	defer os.RemoveAll(dir)
	if err := os.Chmod(dir, 0755); err != nil {
		t.Fatal(err)
	}
	private := filepath.Join(dir, "state")
	if err := os.Mkdir(private, 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(private, "credential"), []byte("private"), 0600); err != nil {
		t.Fatal(err)
	}
	op.home = dir
	s := New(filepath.Join(dir, "cache"), filepath.Join(private, "logs"), op, "laptop", "proj")
	defer s.Close()
	cmd := exec.Command("true")
	if err := runAs(cmd, op); err != nil {
		t.Fatal(err)
	}
	if c := cmd.SysProcAttr.Credential; c == nil || c.Uid != op.uid || c.Gid != op.gid || len(c.Groups) != len(op.groups) {
		t.Fatalf("credential = %+v", c)
	}
	d, _ := artifact(t, s, fmt.Sprintf("#!/bin/sh\nid -u\nif test -r '%s/credential'; then echo exposed; else echo private; fi\nexec sleep 60\n", private))
	applyOK(t, s, d)
	st := awaitStatus(t, s, func(st Status) bool { return strings.Contains(st.LogTail, "private") })
	if !strings.HasPrefix(st.LogTail, fmt.Sprint(op.uid)+"\n") || strings.Contains(st.LogTail, "exposed") {
		t.Fatalf("credential boundary: %+v", st)
	}
	applyOK(t, s)
	s.cacheDir = filepath.Join(private, "cache")
	if rs := s.Apply(context.Background(), []Desired{d}); !strings.Contains(rs[0].Error, "cannot traverse") {
		t.Fatalf("private cache parent accepted: %+v", rs)
	}
}

func TestSharedStickyDirectoryCannotBeTheCache(t *testing.T) {
	s := testSupervisor(t)
	d, _ := artifact(t, s, "#!/bin/sh\nexec sleep 60\n")
	// /tmp is a permitted shared ancestor, but cannot itself be our cache.
	s.cacheDir = "/tmp"
	rs := s.Apply(context.Background(), []Desired{d})
	defer os.Remove(filepath.Join("/tmp", d.SHA256))
	if len(rs) != 1 || !strings.Contains(rs[0].Error, "cache directory") {
		t.Fatalf("shared cache directory accepted: %+v", rs)
	}
}

func TestNewCacheIsTraversableUnderPrivateServiceUmask(t *testing.T) {
	s := testSupervisor(t)
	d, _ := artifact(t, s, "#!/bin/sh\nexec sleep 60\n")
	previous := syscall.Umask(0077)
	defer syscall.Umask(previous)
	applyOK(t, s, d)
	info, err := os.Stat(s.cacheDir)
	if err != nil || info.Mode().Perm() != 0755 {
		t.Fatalf("cache must retain operator traversal under service umask: %v %v", info, err)
	}
}

func TestRunOperatorUsesCleanEnvironment(t *testing.T) {
	s := testSupervisor(t)
	t.Setenv("MESH_SERVICE_PRIVATE_TEST", "not-for-workers")
	cmd := exec.Command("/bin/sh", "-c", "test -z \"$MESH_SERVICE_PRIVATE_TEST\" && test \"$HOME\" = \"$PWD\"")
	if err := RunOperator(context.Background(), cmd, s.operator); err != nil {
		t.Fatalf("operator command inherited service environment or directory: %v", err)
	}
}

func TestRunOperatorClosesInheritedPipes(t *testing.T) {
	s := testSupervisor(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	cmd := exec.Command("/bin/sh", "-c", "sleep 30 &")
	var out bytes.Buffer
	cmd.Stdout = &out
	cmd.Stderr = &out
	started := time.Now()
	_ = RunOperator(ctx, cmd, s.operator)
	if elapsed := time.Since(started); elapsed >= 4*time.Second {
		t.Fatalf("waited for descendant-held pipes until context deadline: %s", elapsed)
	}
}
