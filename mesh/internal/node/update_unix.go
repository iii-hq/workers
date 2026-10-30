//go:build !windows

package node

import (
	"os"
	"path/filepath"
)

func updateTemp(path string) (*os.File, error) {
	return os.CreateTemp(filepath.Dir(path), ".mesh-update-*")
}

func publishUpdatePermissions(f *os.File) error { return f.Chmod(0755) }
