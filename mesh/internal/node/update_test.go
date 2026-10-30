package node

import (
	"context"
	"crypto/sha256"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
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
