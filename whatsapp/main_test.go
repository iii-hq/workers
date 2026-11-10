package main

import (
	"os"
	"path/filepath"
	"testing"
)

func TestMain(m *testing.M) {
	// macOS keeps its temp dir under /var, a symlink, and the inbox refuses any
	// symlink in its path; resolve it once so every t.TempDir() is accepted.
	if dir, err := filepath.EvalSymlinks(os.TempDir()); err == nil {
		os.Setenv("TMPDIR", dir)
	}
	os.Exit(m.Run())
}
