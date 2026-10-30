package install

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

func installerFixture(t *testing.T, mode string) (*exec.Cmd, string, string) {
	t.Helper()
	if _, err := exec.LookPath("jq"); err != nil {
		t.Skip("installer requires jq")
	}
	dir := t.TempDir()
	payload := []byte("#!/bin/sh\nexit 0\n")
	sum := sha256.Sum256(payload)
	digest := hex.EncodeToString(sum[:])
	artifactURL := "https://artifacts.example/mesh"
	if mode == "digest" {
		digest = strings.Repeat("0", 64)
	}
	if mode == "http" {
		artifactURL = "http://artifacts.example/mesh"
	}
	version := "0.1.0"
	if mode == "version" {
		version = "0.2.0"
	}
	entry := map[string]any{"name": "mesh", "version": version, "type": "binary", "binaries": map[string]any{"x86_64-unknown-linux-gnu": map[string]string{"url": artifactURL, "sha256": digest}}}
	graph := []any{entry}
	if mode == "duplicate" {
		graph = append(graph, entry)
	}
	fixture, _ := json.Marshal(map[string]any{"graph": graph})
	os.WriteFile(filepath.Join(dir, "response"), fixture, 0600)
	os.WriteFile(filepath.Join(dir, "artifact"), payload, 0600)
	scripts := map[string]string{
		"uname": "#!/bin/sh\ncase \"$1\" in -s) echo Linux;; -m) echo x86_64;; esac\n",
		"curl":  "#!/bin/sh\nout=\nresolve=no\nwhile [ $# -gt 0 ]; do\n case \"$1\" in --output) shift; out=$1;; https://api.workers.iii.dev/resolve) resolve=yes;; --data) shift; printf '%s' \"$1\" > \"$FIXTURE/request\";; esac\n shift\ndone\nif [ \"$resolve\" = yes ]; then cat \"$FIXTURE/response\"; else cp \"$FIXTURE/artifact\" \"$out\"; fi\n",
		"sudo":  "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$FIXTURE/sudo-args\"\n",
	}
	for name, text := range scripts {
		os.WriteFile(filepath.Join(dir, name), []byte(text), 0700)
	}
	args := []string{"install.sh"}
	if mode == "dry" {
		args = append(args, "--dry-run")
	}
	args = append(args, "0.1.0", "https://hub.example", "ABCD-EFGH")
	cmd := exec.Command("sh", args...)
	cmd.Env = append(os.Environ(), "PATH="+dir+":"+os.Getenv("PATH"), "FIXTURE="+dir)
	return cmd, dir, artifactURL
}
func TestInstallerResolvesExactTargetAndDryRun(t *testing.T) {
	cmd, dir, url := installerFixture(t, "dry")
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("%s: %v", out, err)
	}
	if strings.TrimSpace(string(out)) != url {
		t.Fatal(string(out))
	}
	var req map[string]string
	b, _ := os.ReadFile(filepath.Join(dir, "request"))
	json.Unmarshal(b, &req)
	if req["worker"] != "mesh" || req["version"] != "=0.1.0" || req["target"] != "x86_64-unknown-linux-gnu" {
		t.Fatal(req)
	}
	if _, err := os.Stat(filepath.Join(dir, "sudo-args")); !os.IsNotExist(err) {
		t.Fatal("dry run elevated")
	}
}
func TestInstallerVerifiesBeforeSudo(t *testing.T) {
	for _, mode := range []string{"valid", "digest", "http", "version", "duplicate"} {
		t.Run(mode, func(t *testing.T) {
			cmd, dir, _ := installerFixture(t, mode)
			out, err := cmd.CombinedOutput()
			b, readErr := os.ReadFile(filepath.Join(dir, "sudo-args"))
			if mode == "valid" {
				if err != nil {
					t.Fatalf("%s: %v", out, err)
				}
				if readErr != nil || !strings.HasSuffix(string(b), "join\nhttps://hub.example\nABCD-EFGH\n") {
					t.Fatal(string(b), readErr)
				}
			} else {
				if err == nil {
					t.Fatal("accepted invalid resolution/download")
				}
				if !os.IsNotExist(readErr) {
					t.Fatal("elevated invalid artifact")
				}
			}
		})
	}
}
