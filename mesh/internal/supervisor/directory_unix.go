//go:build !windows

package supervisor

import (
	"errors"
	"os"
	"path/filepath"
)

// Canonical paths keep trusted system aliases (such as macOS /var) usable.
// Every directory is checked before creation beneath it; existing modes stay
// untouched so selecting a secret-state parent cannot make it public.
func secureDirectory(path string, mode os.FileMode, operator *Operator) (string, error) {
	if !filepath.IsAbs(path) {
		return "", errors.New("supervisor directory must be absolute")
	}
	path = filepath.Clean(path)
	if info, err := os.Lstat(path); err == nil {
		if info.Mode()&os.ModeSymlink != 0 {
			if err = trustedOwner(info); err != nil {
				return "", err
			}
			path, err = filepath.EvalSymlinks(path)
			if err != nil {
				return "", err
			}
		}
	} else if !errors.Is(err, os.ErrNotExist) {
		return "", err
	}
	parent := filepath.Dir(path)
	if parent != path {
		canonical, err := secureDirectory(parent, mode, operator)
		if err != nil {
			return "", err
		}
		path = filepath.Join(canonical, filepath.Base(path))
	}
	info, err := os.Lstat(path)
	if errors.Is(err, os.ErrNotExist) {
		if err = os.Mkdir(path, mode); err != nil {
			return "", err
		}
		// A private service umask must not make new cache directories unreachable.
		if err = os.Chmod(path, mode); err != nil {
			return "", err
		}
		info, err = os.Lstat(path)
	}
	if err != nil {
		return "", err
	}
	if err = checkDirectory(path, info, operator); err != nil {
		return "", err
	}
	return path, nil
}

func protectArtifact(f *os.File) error               { return f.Chmod(0555) }
func checkArtifact(_ string, info os.FileInfo) error { return trustedOwner(info) }
func checkLog(_ string, info os.FileInfo) error      { return trustedOwner(info) }

func createArtifactTemp(dir string) (*os.File, error) { return os.CreateTemp(dir, ".download-*") }
