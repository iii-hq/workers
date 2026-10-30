package service

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestSystemdUnit(t *testing.T) {
	text := SystemdUnit("/usr/local/bin/mesh")
	for _, want := range []string{"ExecStart=/usr/local/bin/mesh node", "Restart=always", "KillMode=control-group"} {
		if !strings.Contains(text, want) {
			t.Fatalf("missing %q in %s", want, text)
		}
	}
}
func TestLaunchdPlist(t *testing.T) {
	text := LaunchdPlist("/usr/local/bin/mesh", "/private/log")
	for _, want := range []string{"<key>KeepAlive</key><true/>", "<string>node</string>", "<key>RunAtLoad</key><true/>"} {
		if !strings.Contains(text, want) {
			t.Fatalf("missing %q", want)
		}
	}
}
func TestInstallAndUninstall(t *testing.T) {
	for _, platform := range []string{"linux", "darwin"} {
		t.Run(platform, func(t *testing.T) {
			dir := t.TempDir()
			src := filepath.Join(dir, "source")
			os.WriteFile(src, []byte("binary"), 0600)
			paths := Paths{Bin: filepath.Join(dir, "bin", "mesh"), State: filepath.Join(dir, "private"), Unit: filepath.Join(dir, "service"), Cache: filepath.Join(dir, "cache")}
			var calls []string
			m := manager{platform: platform, paths: paths, run: func(cmd string, args ...string) error {
				calls = append(calls, cmd+" "+strings.Join(args, " "))
				if cmd == "launchctl" && args[0] == "print" {
					return errors.New("absent")
				}
				return nil
			}}
			if err := m.install(src); err != nil {
				t.Fatal(err)
			}
			b, err := os.ReadFile(paths.Bin)
			if err != nil || string(b) != "binary" {
				t.Fatalf("binary: %q %v", b, err)
			}
			st, _ := os.Stat(paths.Bin)
			if st.Mode().Perm() != 0755 {
				t.Fatal(st.Mode())
			}
			if platform == "linux" && strings.Join(calls, "|") != "systemctl daemon-reload|systemctl enable iii-mesh|systemctl restart iii-mesh" {
				t.Fatal(calls)
			}
			if err := m.uninstall(); err != nil {
				t.Fatal(err)
			}
			if _, err := os.Stat(paths.Unit); !os.IsNotExist(err) {
				t.Fatal("unit remains", err)
			}
		})
	}
}
func TestStopFailureRetainsService(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "service")
	os.WriteFile(path, []byte("unit"), 0600)
	m := manager{platform: "linux", paths: Paths{Unit: path}, run: func(string, ...string) error { return errors.New("stop failed") }}
	if m.uninstall() == nil {
		t.Fatal("accepted failed stop")
	}
	if _, err := os.Stat(path); err != nil {
		t.Fatal(err)
	}
}
