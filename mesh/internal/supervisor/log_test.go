//go:build linux || darwin

package supervisor

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestLogsRotateAtTenMiBKeepingThreeFiles(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "logs")
	log, err := openLog(dir, "ide")
	if err != nil {
		t.Fatal(err)
	}
	defer log.Close()
	for _, b := range []byte{'a', 'b', 'c', 'd'} {
		block := bytes.Repeat([]byte{b}, 10*1024*1024)
		if n, err := log.Write(block); err != nil || n != len(block) {
			t.Fatalf("write=%d %v", n, err)
		}
	}
	if err := log.Close(); err != nil {
		t.Fatal(err)
	}
	files, err := os.ReadDir(dir)
	if err != nil || len(files) != 3 {
		t.Fatalf("retained logs = %v %v", files, err)
	}
	for suffix, want := range map[string]byte{"": 'd', ".1": 'c', ".2": 'b'} {
		data, err := os.ReadFile(filepath.Join(dir, "ide.log"+suffix))
		if err != nil || len(data) != 10*1024*1024 || data[0] != want || data[len(data)-1] != want {
			t.Fatalf("rotation %s: size %d err %v", suffix, len(data), err)
		}
		st, err := os.Stat(filepath.Join(dir, "ide.log"+suffix))
		if err != nil || st.Mode().Perm() != 0600 {
			t.Fatalf("private log %s: %v %v", suffix, st, err)
		}
	}
	if got := log.tail(); len(got) != 16*1024 || strings.Trim(got, "d") != "" {
		t.Fatalf("log tail size/content = %d", len(got))
	}
	reopened, err := openLog(dir, "ide")
	if err != nil {
		t.Fatal(err)
	}
	defer reopened.Close()
	if reopened.tail() != log.tail() {
		t.Fatal("log tail lost on reopen")
	}
}
func TestLogRefusesSymlinkAndPublicDirectory(t *testing.T) {
	for _, public := range []bool{false, true} {
		t.Run(map[bool]string{false: "symlink", true: "public"}[public], func(t *testing.T) {
			dir := filepath.Join(t.TempDir(), "logs")
			mode := os.FileMode(0700)
			if public {
				mode = 0755
			}
			if err := os.Mkdir(dir, mode); err != nil {
				t.Fatal(err)
			}
			target := filepath.Join(t.TempDir(), "secret")
			if err := os.WriteFile(target, []byte("unchanged"), 0600); err != nil {
				t.Fatal(err)
			}
			if !public {
				if err := os.Symlink(target, filepath.Join(dir, "ide.log")); err != nil {
					t.Fatal(err)
				}
			}
			if log, err := openLog(dir, "ide"); err == nil {
				log.Close()
				t.Fatal("unsafe log accepted")
			}
			got, err := os.ReadFile(target)
			if err != nil || string(got) != "unchanged" {
				t.Fatalf("target modified: %q %v", got, err)
			}
		})
	}
}
