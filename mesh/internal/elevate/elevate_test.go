//go:build linux || darwin

package elevate

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestRunPreservesArgumentsAndExit(t *testing.T) {
	dir := t.TempDir()
	out := filepath.Join(dir, "args")
	t.Setenv("OUTPUT", out)
	os.WriteFile(filepath.Join(dir, "sudo"), []byte("#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$OUTPUT\"\nexit 7\n"), 0700)
	t.Setenv("PATH", dir)
	if got := Run("join", "https://host/path with space", "ABCD-EFGH"); got != 7 {
		t.Fatal(got)
	}
	b, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasSuffix(string(b), "join\nhttps://host/path with space\nABCD-EFGH\n") {
		t.Fatal(string(b))
	}
}
func TestOperatorRefusesRoot(t *testing.T) {
	for _, name := range []string{"", "root"} {
		if _, err := operatorName(0, "root", name); err == nil {
			t.Fatal("accepted", name)
		}
	}
	if got, err := operatorName(0, "root", "alice"); err != nil || got != "alice" {
		t.Fatal(got, err)
	}
	if got, err := operatorName(1000, "alice", "mallory"); err != nil || got != "alice" {
		t.Fatal(got, err)
	}
}
