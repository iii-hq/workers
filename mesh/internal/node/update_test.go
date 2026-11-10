package node

import (
	"archive/tar"
	"bytes"
	"compress/gzip"
	"context"
	"crypto/sha256"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"runtime"
	"slices"
	"strings"
	"testing"
	"time"
)

func TestUpdateRefusesBadSHA(t *testing.T) {
	binary := filepath.Join(t.TempDir(), "mesh")
	os.WriteFile(binary, []byte("old"), 0755)
	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { fmt.Fprint(w, "new binary") }))
	defer server.Close()
	u := Updater{BinaryPath: binary, Client: server.Client()}
	if err := u.Apply(context.Background(), UpdateRequest{Version: "0.2.0", ArtifactURL: server.URL, SHA256: strings.Repeat("0", 64)}); err == nil {
		t.Fatal("bad digest accepted")
	}
	got, _ := os.ReadFile(binary)
	if string(got) != "old" {
		t.Fatalf("original replaced: %q", got)
	}
	entries, _ := os.ReadDir(filepath.Dir(binary))
	if len(entries) != 1 {
		t.Fatalf("temporary download leaked: %v", entries)
	}
}

func TestUpdateAtomicReplacement(t *testing.T) {
	binary := filepath.Join(t.TempDir(), "mesh")
	os.WriteFile(binary, []byte("old"), 0755)
	content := []byte("verified replacement")
	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { w.Write(content) }))
	defer server.Close()
	u := Updater{BinaryPath: binary, Client: server.Client()}
	err := u.Apply(t.Context(), UpdateRequest{Version: "0.2.0", ArtifactURL: server.URL, SHA256: fmt.Sprintf("%x", sha256.Sum256(content))})
	if err != nil {
		t.Fatal(err)
	}
	got, _ := os.ReadFile(binary)
	info, _ := os.Stat(binary)
	if string(got) != string(content) || info.Mode().Perm() != 0755 {
		t.Fatalf("replacement %q mode %v", got, info.Mode())
	}
	assertNoUpdateLeftovers(t, binary)
}

// updatePackage builds an in-memory gzip tar holding one executable per name.
func updatePackage(t *testing.T, names ...string) []byte {
	t.Helper()
	var buffer bytes.Buffer
	compressed := gzip.NewWriter(&buffer)
	archive := tar.NewWriter(compressed)
	for _, name := range names {
		body := "binary " + name
		if err := archive.WriteHeader(&tar.Header{Name: name, Typeflag: tar.TypeReg, Mode: 0755, Size: int64(len(body))}); err != nil {
			t.Fatal(err)
		}
		if _, err := archive.Write([]byte(body)); err != nil {
			t.Fatal(err)
		}
	}
	if err := archive.Close(); err != nil {
		t.Fatal(err)
	}
	if err := compressed.Close(); err != nil {
		t.Fatal(err)
	}
	return buffer.Bytes()
}

// applyUpdate serves content with its correct digest and applies it over a binary holding "old".
func applyUpdate(t *testing.T, content []byte) (string, error) {
	t.Helper()
	binary := filepath.Join(t.TempDir(), "mesh")
	os.WriteFile(binary, []byte("old"), 0755)
	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { w.Write(content) }))
	t.Cleanup(server.Close)
	u := Updater{BinaryPath: binary, Client: server.Client()}
	return binary, u.Apply(t.Context(), UpdateRequest{Version: "0.2.0", ArtifactURL: server.URL, SHA256: fmt.Sprintf("%x", sha256.Sum256(content))})
}

func assertNoUpdateLeftovers(t *testing.T, binary string) {
	t.Helper()
	if entries, _ := os.ReadDir(filepath.Dir(binary)); len(entries) != 1 {
		t.Fatalf("update left files beside the binary: %v", entries)
	}
}

func TestUpdateInstallsExecutableFromPackage(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("gzip tar packages are POSIX-only")
	}
	binary, err := applyUpdate(t, updatePackage(t, "mesh"))
	if err != nil {
		t.Fatal(err)
	}
	got, _ := os.ReadFile(binary)
	info, _ := os.Stat(binary)
	if string(got) != "binary mesh" || info.Mode().Perm() != 0755 {
		t.Fatalf("replacement %q mode %v", got, info.Mode())
	}
	assertNoUpdateLeftovers(t, binary)
}

func TestUpdateRefusesUnrunnableArtifacts(t *testing.T) {
	for name, content := range map[string][]byte{
		"traversal package":            updatePackage(t, "../mesh"),
		"zip package":                  []byte("PK\x03\x04mesh.exe"),
		"executable with another name": updatePackage(t, "install.sh"),
	} {
		t.Run(name, func(t *testing.T) {
			binary, err := applyUpdate(t, content)
			if err == nil || !strings.Contains(err.Error(), "mesh package") {
				t.Fatalf("unrunnable artifact accepted: %v", err)
			}
			if got, _ := os.ReadFile(binary); string(got) != "old" {
				t.Fatalf("original replaced: %q", got)
			}
			assertNoUpdateLeftovers(t, binary)
		})
	}
}

func TestUpdateRefusesZipBeforeDownloading(t *testing.T) {
	binary := filepath.Join(t.TempDir(), "mesh")
	os.WriteFile(binary, []byte("old"), 0755)
	// The body breaks after the zip magic, so only a refusal read from the first bytes returns the package error.
	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write([]byte("PK\x03\x04"))
		w.(http.Flusher).Flush()
		panic(http.ErrAbortHandler)
	}))
	defer server.Close()
	u := Updater{BinaryPath: binary, Client: server.Client()}
	err := u.Apply(t.Context(), UpdateRequest{Version: "0.2.0", ArtifactURL: server.URL, SHA256: strings.Repeat("0", 64)})
	if err == nil || !strings.Contains(err.Error(), "mesh package") {
		t.Fatalf("zip not refused from its first bytes: %v", err)
	}
	if got, _ := os.ReadFile(binary); string(got) != "old" {
		t.Fatalf("original replaced: %q", got)
	}
	assertNoUpdateLeftovers(t, binary)
}

func TestUpdateReclaimsInterruptedUpdateFiles(t *testing.T) {
	binary := filepath.Join(t.TempDir(), "mesh")
	dir := filepath.Dir(binary)
	os.WriteFile(binary, []byte("old"), 0755)
	// A killed update left a download and a partial extraction; .mesh-update-3 is another update still writing.
	os.WriteFile(filepath.Join(dir, ".mesh-update-1"), []byte("partial"), 0600)
	os.Mkdir(filepath.Join(dir, ".mesh-update-2"), 0700)
	os.WriteFile(filepath.Join(dir, ".mesh-update-2", "mesh"), []byte("partial"), 0400)
	os.WriteFile(filepath.Join(dir, ".mesh-update-3"), []byte("running"), 0600)
	stale := time.Now().Add(-2 * time.Hour)
	for _, name := range []string{".mesh-update-1", ".mesh-update-2"} {
		os.Chtimes(filepath.Join(dir, name), stale, stale)
	}
	content := []byte("verified replacement")
	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { w.Write(content) }))
	defer server.Close()
	u := Updater{BinaryPath: binary, Client: server.Client()}
	if err := u.Apply(t.Context(), UpdateRequest{Version: "0.2.0", ArtifactURL: server.URL, SHA256: fmt.Sprintf("%x", sha256.Sum256(content))}); err != nil {
		t.Fatal(err)
	}
	entries, _ := os.ReadDir(dir)
	var names []string
	for _, entry := range entries {
		names = append(names, entry.Name())
	}
	if !slices.Equal(names, []string{".mesh-update-3", "mesh"}) {
		t.Fatalf("entries beside the binary: %v", names)
	}
}

func TestUpdateRefusesHTTPAndRedirectDowngrade(t *testing.T) {
	binary := filepath.Join(t.TempDir(), "mesh")
	os.WriteFile(binary, []byte("old"), 0755)
	served := false
	insecure := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { served = true; fmt.Fprint(w, "bad") }))
	defer insecure.Close()
	secure := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { http.Redirect(w, r, insecure.URL, http.StatusFound) }))
	defer secure.Close()
	for _, url := range []string{insecure.URL, secure.URL} {
		u := Updater{BinaryPath: binary, Client: secure.Client()}
		if err := u.Apply(t.Context(), UpdateRequest{Version: "v", ArtifactURL: url, SHA256: strings.Repeat("0", 64)}); err == nil {
			t.Fatal("insecure download accepted")
		}
	}
	if served {
		t.Fatal("followed insecure redirect")
	}
}

func TestReplacementWithRunningExecutableBackup(t *testing.T) {
	dir := t.TempDir()
	binary := filepath.Join(dir, "mesh.exe")
	download := filepath.Join(dir, "download")
	os.WriteFile(binary, []byte("running"), 0755)
	os.WriteFile(download, []byte("replacement"), 0755)
	if err := replaceRunningBinary(download, binary); err != nil {
		t.Fatal(err)
	}
	got, _ := os.ReadFile(binary)
	if string(got) != "replacement" {
		t.Fatalf("replacement %q", got)
	}
	os.WriteFile(binary, []byte("retained"), 0755)
	if err := replaceRunningBinary(filepath.Join(dir, "missing"), binary); err == nil {
		t.Fatal("missing download accepted")
	}
	got, _ = os.ReadFile(binary)
	if string(got) != "retained" {
		t.Fatalf("rollback %q", got)
	}
}
