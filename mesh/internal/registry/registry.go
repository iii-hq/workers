package registry

import (
	"bytes"
	"context"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
)

const DefaultBase = "https://api.workers.iii.dev"

var (
	ErrNotBinary   = errors.New("worker is not a binary")
	ErrNoTarget    = errors.New("worker has no binary for target")
	ErrInsecureURL = errors.New("artifact URL must be a complete HTTPS URL")
	ErrBadDigest   = errors.New("artifact SHA256 must have 64 hex digits")
)

type Artifact struct {
	Version string
	URL     string
	SHA256  string
}

// Resolve uses DefaultBase and http.DefaultClient when base and c are unset.
func Resolve(ctx context.Context, c *http.Client, base, worker, versionRange, target string) (Artifact, error) {
	if base == "" {
		base = DefaultBase
	}
	if c == nil {
		c = http.DefaultClient
	}
	body, err := json.Marshal(struct {
		Worker  string `json:"worker"`
		Version string `json:"version"`
		Target  string `json:"target"`
	}{worker, versionRange, target})
	if err != nil {
		return Artifact{}, err
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, strings.TrimRight(base, "/")+"/resolve", bytes.NewReader(body))
	if err != nil {
		return Artifact{}, err
	}
	req.Header.Set("Content-Type", "application/json")
	resp, err := c.Do(req)
	if err != nil {
		return Artifact{}, err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return Artifact{}, fmt.Errorf("registry resolve: HTTP %s", resp.Status)
	}
	var result struct {
		Graph []struct {
			Name     string `json:"name"`
			Version  string `json:"version"`
			Type     string `json:"type"`
			Binaries map[string]struct {
				URL    string `json:"url"`
				SHA256 string `json:"sha256"`
			} `json:"binaries"`
		} `json:"graph"`
	}
	decoder := json.NewDecoder(resp.Body)
	if err := decoder.Decode(&result); err != nil {
		return Artifact{}, fmt.Errorf("registry resolve: %w", err)
	}
	if err := decoder.Decode(new(any)); err != io.EOF {
		return Artifact{}, errors.New("registry resolve: trailing response data")
	}
	index := -1
	for i, entry := range result.Graph {
		if entry.Name == worker {
			if index != -1 {
				return Artifact{}, fmt.Errorf("registry resolve: duplicate entry for %q", worker)
			}
			index = i
		}
	}
	if index == -1 {
		return Artifact{}, fmt.Errorf("registry resolve: no entry for %q", worker)
	}
	entry := result.Graph[index]
	if entry.Version == "" {
		return Artifact{}, fmt.Errorf("registry resolve: missing version for %q", worker)
	}
	if entry.Type != "binary" {
		return Artifact{}, ErrNotBinary
	}
	binary, ok := entry.Binaries[target]
	if !ok {
		return Artifact{}, ErrNoTarget
	}
	u, err := url.Parse(binary.URL)
	if err != nil || u.Scheme != "https" || u.Hostname() == "" {
		return Artifact{}, ErrInsecureURL
	}
	if _, err := hex.DecodeString(binary.SHA256); len(binary.SHA256) != 64 || err != nil {
		return Artifact{}, ErrBadDigest
	}
	return Artifact{Version: entry.Version, URL: binary.URL, SHA256: binary.SHA256}, nil
}

// TargetFor returns an empty string for unsupported OS, architecture, or libc combinations.
func TargetFor(goos, goarch string, musl bool) string {
	var arch string
	switch goarch {
	case "amd64":
		arch = "x86_64"
	case "arm64":
		arch = "aarch64"
	default:
		return ""
	}
	switch goos {
	case "linux":
		if musl {
			if goarch != "amd64" {
				return ""
			}
			return arch + "-unknown-linux-musl"
		}
		return arch + "-unknown-linux-gnu"
	case "darwin":
		return arch + "-apple-darwin"
	case "windows":
		return arch + "-pc-windows-msvc"
	default:
		return ""
	}
}
