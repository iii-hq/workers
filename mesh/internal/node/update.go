package node

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
	"runtime"
	"strings"
	"sync"

	"github.com/iii-hq/workers/mesh/internal/registry"
)

type UpdateRequest struct {
	Version     string `json:"version"`
	ArtifactURL string `json:"artifact_url"`
	SHA256      string `json:"sha256"`
}

type Updater struct {
	BinaryPath string
	Client     *http.Client
	mu         sync.Mutex
}

func (u *Updater) Apply(ctx context.Context, r UpdateRequest) error {
	u.mu.Lock()
	defer u.mu.Unlock()
	if r.Version == "" || u.BinaryPath == "" {
		return errors.New("update requires version and binary path")
	}
	parsed, err := url.Parse(r.ArtifactURL)
	if err != nil || parsed.Scheme != "https" || parsed.Hostname() == "" || parsed.User != nil {
		return registry.ErrInsecureURL
	}
	if _, err := hex.DecodeString(r.SHA256); err != nil || len(r.SHA256) != 64 {
		return registry.ErrBadDigest
	}
	client := http.DefaultClient
	if u.Client != nil {
		client = u.Client
	}
	copyClient := *client
	previousRedirect := client.CheckRedirect
	copyClient.CheckRedirect = func(req *http.Request, via []*http.Request) error {
		if req.URL.Scheme != "https" {
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
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, r.ArtifactURL, nil)
	if err != nil {
		return err
	}
	response, err := copyClient.Do(req)
	if err != nil {
		return err
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusOK {
		return fmt.Errorf("download mesh: HTTP %s", response.Status)
	}
	file, err := os.CreateTemp(filepath.Dir(u.BinaryPath), ".mesh-update-*")
	if err != nil {
		return err
	}
	defer os.Remove(file.Name())
	defer file.Close()
	digest := sha256.New()
	if _, err = io.Copy(io.MultiWriter(file, digest), response.Body); err != nil {
		return err
	}
	if !strings.EqualFold(hex.EncodeToString(digest.Sum(nil)), r.SHA256) {
		return errors.New("mesh artifact SHA256 mismatch")
	}
	if err = ctx.Err(); err != nil {
		return err
	}
	if err = file.Chmod(0755); err != nil {
		return err
	}
	if err = file.Sync(); err != nil {
		return err
	}
	if err = file.Close(); err != nil {
		return err
	}
	if runtime.GOOS == "windows" {
		return replaceRunningBinary(file.Name(), u.BinaryPath)
	}
	return os.Rename(file.Name(), u.BinaryPath)
}

// Windows cannot replace a mapped executable in place. Move it aside first;
// retain at most one backup if this process still prevents its deletion.
func replaceRunningBinary(download, binary string) error {
	backup := binary + ".previous"
	if err := os.Remove(backup); err != nil && !errors.Is(err, os.ErrNotExist) {
		return err
	}
	if err := os.Rename(binary, backup); err != nil {
		return err
	}
	if err := os.Rename(download, binary); err != nil {
		return errors.Join(err, os.Rename(backup, binary))
	}
	_ = os.Remove(backup)
	return nil
}
