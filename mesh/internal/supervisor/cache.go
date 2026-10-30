package supervisor

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strings"

	"github.com/iii-hq/workers/mesh/internal/registry"
)

func validateArtifact(d Desired) error {
	u, err := url.Parse(d.URL)
	if err != nil || u.Scheme != "https" || u.Hostname() == "" || u.User != nil {
		return registry.ErrInsecureURL
	}
	if _, err := hex.DecodeString(d.SHA256); err != nil || len(d.SHA256) != 64 {
		return registry.ErrBadDigest
	}
	return nil
}
func (s *Supervisor) artifact(ctx context.Context, d Desired) (string, error) {
	dir, err := secureDirectory(s.cacheDir, 0755, &s.operator)
	if err != nil {
		return "", fmt.Errorf("worker cache: %w", err)
	}
	info, err := os.Stat(dir)
	if err != nil {
		return "", err
	}
	if info.Mode().Perm()&0022 != 0 || info.Mode()&os.ModeSticky != 0 {
		return "", errors.New("worker cache directory must not be shared or writable by another account")
	}
	path := filepath.Join(dir, d.SHA256)
	if _, err = os.Lstat(path); err == nil {
		return path, verifyArtifact(path, d.SHA256)
	} else if !errors.Is(err, os.ErrNotExist) {
		return "", err
	}
	client := *s.client
	previousRedirect := client.CheckRedirect
	client.CheckRedirect = func(req *http.Request, via []*http.Request) error {
		if req.URL.Scheme != "https" || req.URL.User != nil {
			return registry.ErrInsecureURL
		}
		if previousRedirect != nil {
			return previousRedirect(req, via)
		}
		if len(via) >= 10 {
			return errors.New("too many artifact redirects")
		}
		return nil
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, d.URL, nil)
	if err != nil {
		return "", err
	}
	response, err := client.Do(req)
	if err != nil {
		return "", err
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusOK {
		return "", fmt.Errorf("download %s: HTTP %s", d.Name, response.Status)
	}
	file, err := os.CreateTemp(dir, ".download-*")
	if err != nil {
		return "", err
	}
	defer os.Remove(file.Name())
	defer file.Close()
	hash := sha256.New()
	if _, err = io.Copy(io.MultiWriter(file, hash), response.Body); err != nil {
		return "", err
	}
	actual := hex.EncodeToString(hash.Sum(nil))
	if actual != d.SHA256 {
		return "", fmt.Errorf("worker SHA256 mismatch: expected %s, got %s", d.SHA256, actual)
	}
	if err = ctx.Err(); err != nil {
		return "", err
	}
	if err = file.Chmod(0555); err != nil {
		return "", err
	}
	if err = file.Sync(); err != nil {
		return "", err
	}
	if err = file.Close(); err != nil {
		return "", err
	}
	if err = os.Rename(file.Name(), path); err != nil {
		return "", err
	}
	return path, nil
}
func verifyArtifact(path, digest string) error {
	info, err := os.Lstat(path)
	if err != nil {
		return err
	}
	if !info.Mode().IsRegular() || info.Mode().Perm() != 0555 {
		return fmt.Errorf("unsafe cached artifact permissions: %s", path)
	}
	if err = trustedOwner(info); err != nil {
		return err
	}
	file, err := os.Open(path)
	if err != nil {
		return err
	}
	defer file.Close()
	hash := sha256.New()
	if _, err = io.Copy(hash, file); err != nil {
		return err
	}
	actual := hex.EncodeToString(hash.Sum(nil))
	if !strings.EqualFold(actual, digest) {
		return fmt.Errorf("worker SHA256 mismatch: expected %s, got %s", digest, actual)
	}
	return nil
}

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
