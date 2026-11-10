package node

import (
	"bufio"
	"bytes"
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
	"time"

	"github.com/iii-hq/workers/mesh/internal/registry"
	"github.com/iii-hq/workers/mesh/internal/supervisor"
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
	// The registry ships mesh as a package: read the format from the content as the supervisor
	// does, and refuse one this node cannot unpack before downloading it, since the hub retries.
	body := bufio.NewReader(response.Body)
	magic, err := body.Peek(4)
	if err != nil && err != io.EOF {
		return err
	}
	packaged := bytes.HasPrefix(magic, []byte{0x1f, 0x8b})
	if bytes.HasPrefix(magic, []byte("PK\x03\x04")) || packaged && runtime.GOOS == "windows" {
		// shortcut: zip packages (the registry's Windows format) and gzip tar on Windows are refused, add zip extraction with winsecurity protection when Windows nodes self-update from the registry.
		return errors.New("mesh package: zip, and gzip tar on Windows, are not supported")
	}
	file, err := updateTemp(u.BinaryPath)
	if err != nil {
		return err
	}
	defer os.Remove(file.Name())
	defer file.Close()
	reclaimUpdateLeftovers(u.BinaryPath)
	digest := sha256.New()
	if _, err = io.Copy(io.MultiWriter(file, digest), body); err != nil {
		return err
	}
	if !strings.EqualFold(hex.EncodeToString(digest.Sum(nil)), r.SHA256) {
		return errors.New("mesh artifact SHA256 mismatch")
	}
	if err = ctx.Err(); err != nil {
		return err
	}
	if packaged {
		// Extract beside the binary so the final rename cannot cross a file system.
		dir, err := os.MkdirTemp(filepath.Dir(u.BinaryPath), ".mesh-update-*")
		if err != nil {
			return err
		}
		defer os.RemoveAll(dir)
		if _, err = file.Seek(0, io.SeekStart); err != nil {
			return err
		}
		name, err := supervisor.Unpack(file, dir, "mesh", true)
		if err == nil && name != "mesh" {
			// Unpack falls back to the only executable for workers named apart from their binary; mesh always ships as mesh.
			err = fmt.Errorf("no executable named mesh, only %s", name)
		}
		if err != nil {
			return fmt.Errorf("mesh package: %w", err)
		}
		if file, err = os.Open(filepath.Join(dir, name)); err != nil {
			return err
		}
		defer file.Close()
	}
	if err = publishUpdatePermissions(file); err != nil {
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

// reclaimUpdateLeftovers removes the downloads and extractions an update killed
// mid-way left beside the binary; their deferred cleanup never ran.
// An entry written within the last hour may belong to an update still running in another process.
// shortcut: leftovers of a kill stay until an update starts more than an hour later, reclaim at node start if they pile up.
func reclaimUpdateLeftovers(binary string) {
	dir := filepath.Dir(binary)
	entries, _ := os.ReadDir(dir)
	for _, entry := range entries {
		if !strings.HasPrefix(entry.Name(), ".mesh-update-") {
			continue
		}
		if info, err := entry.Info(); err == nil && time.Since(info.ModTime()) > time.Hour {
			_ = os.RemoveAll(filepath.Join(dir, entry.Name()))
		}
	}
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
