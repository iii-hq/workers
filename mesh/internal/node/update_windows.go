package node

import (
	"os"
	"path/filepath"

	"github.com/iii-hq/workers/mesh/internal/winsecurity"
)

func updateTemp(path string) (*os.File, error) {
	if err := winsecurity.Check(path, true); err != nil {
		return nil, err
	}
	return winsecurity.CreatePrivateTemp(filepath.Dir(path), ".mesh-update-")
}

func publishUpdatePermissions(f *os.File) error { return winsecurity.Protect(f.Name(), true) }
