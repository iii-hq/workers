//go:build linux || darwin

package supervisor

import (
	"archive/tar"
	"bytes"
	"compress/gzip"
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

type packed struct {
	header tar.Header
	body   string
}

func packedFile(name string, mode int64, body string) packed {
	return packed{tar.Header{Name: name, Typeflag: tar.TypeReg, Mode: mode, Size: int64(len(body))}, body}
}

// packageArchive builds a gzip tar in memory. A last entry that declares more
// bytes than its body ends the archive right after its header.
func packageArchive(t *testing.T, entries ...packed) string {
	t.Helper()
	var buffer bytes.Buffer
	compressed := gzip.NewWriter(&buffer)
	archive := tar.NewWriter(compressed)
	short := false
	for _, entry := range entries {
		if err := archive.WriteHeader(&entry.header); err != nil {
			t.Fatal(err)
		}
		if _, err := archive.Write([]byte(entry.body)); err != nil {
			t.Fatal(err)
		}
		short = entry.header.Size > int64(len(entry.body))
	}
	if err := archive.Close(); err != nil && !short {
		t.Fatal(err)
	}
	if err := compressed.Close(); err != nil {
		t.Fatal(err)
	}
	return buffer.String()
}

func TestPackageIsExtractedOnceAndComparedBeforeEveryStart(t *testing.T) {
	script := "#!/bin/sh\necho \"packaged $(cat \"$(dirname \"$0\")/data\")\"\nexec sleep 60\n"
	for _, tt := range []struct{ name, binary, tampered string }{
		{"binary named after the worker", "ide", "ide"},
		{"only executable", "iii-ide", "planted"},
	} {
		t.Run(tt.name, func(t *testing.T) {
			s := testSupervisor(t)
			entries := []packed{packedFile(tt.binary, 0755, script), packedFile("data", 0644, "companion")}
			if tt.binary == "ide" {
				entries = append(entries, packedFile("helper", 0755, "#!/bin/sh\n"))
			}
			d, count := artifact(t, s, packageArchive(t, entries...))
			applyOK(t, s, d)
			awaitStatus(t, s, func(st Status) bool { return strings.Contains(st.LogTail, "packaged companion") })
			applyOK(t, s)
			dir := filepath.Join(s.cacheDir, d.SHA256+".d")
			for name, mode := range map[string]os.FileMode{tt.binary: 0555, "data": 0444} {
				if info, err := os.Lstat(filepath.Join(dir, name)); err != nil || info.Mode() != mode {
					t.Fatalf("extracted %s: %v %v", name, info, err)
				}
			}
			if cached, err := os.ReadDir(s.cacheDir); err != nil || len(cached) != 2 {
				t.Fatalf("cache must hold the artifact and its package only: %v %v", cached, err)
			}
			applyOK(t, s, d)
			applyOK(t, s)
			path := filepath.Join(dir, tt.tampered)
			os.Remove(path)
			if err := os.WriteFile(path, []byte("#!/bin/sh\nexec sleep 60\n"), 0555); err != nil {
				t.Fatal(err)
			}
			rs := s.Apply(context.Background(), []Desired{d})
			if len(rs) != 1 || !strings.Contains(rs[0].Error, "worker package") || s.Status()[0].PID != 0 {
				t.Fatalf("tampered package executed: %+v", rs)
			}
			if count.Load() != 1 {
				t.Fatalf("package was not reused from the cache: %d downloads", count.Load())
			}
		})
	}
}

func TestPackageRejectsUnsafeArchives(t *testing.T) {
	outside := t.TempDir()
	binary := packedFile("ide", 0755, "#!/bin/sh\nexec sleep 60\n")
	many := []packed{binary}
	for range maxPackageEntries {
		many = append(many, packed{header: tar.Header{Name: "lib/", Typeflag: tar.TypeDir, Mode: 0755}})
	}
	for _, tt := range []struct {
		name, want string
		entries    []packed
	}{
		{"parent traversal", "unsafe path", []packed{binary, packedFile("../escape", 0755, "x")}},
		{"inner traversal", "unsafe path", []packed{binary, packedFile("lib/../escape", 0755, "x")}},
		{"absolute path", "unsafe path", []packed{binary, packedFile(filepath.Join(outside, "escape"), 0755, "x")}},
		{"symlink", "unsupported entry", []packed{{header: tar.Header{Name: "ide", Typeflag: tar.TypeSymlink, Linkname: "/bin/sleep", Mode: 0777}}}},
		{"hard link", "unsupported entry", []packed{binary, {header: tar.Header{Name: "copy", Typeflag: tar.TypeLink, Linkname: "ide", Mode: 0755}}}},
		{"oversize", "bytes", []packed{binary, {header: tar.Header{Name: "big", Typeflag: tar.TypeReg, Mode: 0644, Size: maxPackageBytes}}}},
		{"too many entries", "entries", many},
		{"no executable", "no single executable", []packed{packedFile("data", 0644, "x")}},
	} {
		t.Run(tt.name, func(t *testing.T) {
			s := testSupervisor(t)
			d, _ := artifact(t, s, packageArchive(t, tt.entries...))
			rs := s.Apply(context.Background(), []Desired{d})
			if len(rs) != 1 || !strings.Contains(rs[0].Error, tt.want) {
				t.Fatalf("results = %+v", rs)
			}
			if st := s.Status()[0]; st.State != "error" || st.PID != 0 {
				t.Fatalf("status = %+v", st)
			}
			// Only the verified archive stays: no partial package and nothing beside it.
			if cached, err := os.ReadDir(s.cacheDir); err != nil || len(cached) != 1 || cached[0].Name() != d.SHA256 {
				t.Fatalf("cache = %v %v", cached, err)
			}
			if escaped, err := os.ReadDir(outside); err != nil || len(escaped) != 0 {
				t.Fatalf("escaped = %v %v", escaped, err)
			}
		})
	}
}

func TestPackagedWorkerRestartsFromItsComparedTree(t *testing.T) {
	s := testSupervisor(t)
	script := "#!/bin/sh\necho \"packaged $(cat \"$(dirname \"$0\")/data\")\"\nexit 3\n"
	d, _ := artifact(t, s, packageArchive(t, packedFile("ide", 0755, script), packedFile("data", 0644, "companion")))
	applyOK(t, s, d)
	awaitStatus(t, s, func(st Status) bool { return st.Restarts == 1 && strings.Count(st.LogTail, "packaged companion") == 2 })
	data := filepath.Join(s.cacheDir, d.SHA256+".d", "data")
	os.Remove(data)
	if err := os.WriteFile(data, []byte("changed"), 0444); err != nil {
		t.Fatal(err)
	}
	st := awaitStatus(t, s, func(st Status) bool { return st.Restarts == 2 && strings.Contains(st.Error, "restart: worker package") })
	if strings.Count(st.LogTail, "packaged") != 2 {
		t.Fatalf("changed package restarted: %+v", st)
	}
}

func TestPackageComparesDirectoriesAndRefusesLinks(t *testing.T) {
	for _, tamper := range []string{"symlinked directory", "writable directory"} {
		t.Run(tamper, func(t *testing.T) {
			s := testSupervisor(t)
			d, _ := artifact(t, s, packageArchive(t, packedFile("ide", 0755, "#!/bin/sh\nexec sleep 60\n"), packedFile("lib/x.so", 0644, "library")))
			applyOK(t, s, d)
			applyOK(t, s)
			lib := filepath.Join(s.cacheDir, d.SHA256+".d", "lib")
			if tamper == "writable directory" {
				if err := os.Chmod(lib, 0777); err != nil {
					t.Fatal(err)
				}
			} else {
				// The copy passes every per-file check, so only the link gives it away.
				other := t.TempDir()
				if err := os.WriteFile(filepath.Join(other, "x.so"), []byte("library"), 0444); err != nil {
					t.Fatal(err)
				}
				if err := os.RemoveAll(lib); err != nil {
					t.Fatal(err)
				}
				if err := os.Symlink(other, lib); err != nil {
					t.Fatal(err)
				}
			}
			rs := s.Apply(context.Background(), []Desired{d})
			if len(rs) != 1 || !strings.Contains(rs[0].Error, "unsafe cached worker package") || s.Status()[0].PID != 0 {
				t.Fatalf("tampered package executed: %+v", rs)
			}
		})
	}
}
