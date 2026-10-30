//go:build !windows

package cli

import (
	"errors"
	"os"

	"github.com/iii-hq/workers/mesh/internal/service"
)

func privateDir(dir string) error {
	if err := os.MkdirAll(dir, 0700); err != nil {
		return err
	}
	st, err := os.Lstat(dir)
	if err != nil {
		return err
	}
	if !st.IsDir() || st.Mode()&os.ModeSymlink != 0 {
		return errors.New("node state must be a private directory")
	}
	return os.Chmod(dir, 0700)
}

func checkPrivateState(string, string) error { return nil }
func configurePlatform(a *app) *app          { return a }

func removeInstalled(p service.Paths) error { return removeIfExists(p.Bin) }
