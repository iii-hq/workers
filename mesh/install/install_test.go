package install

import (
	"crypto/ed25519"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"github.com/iii-hq/workers/mesh/internal/cli"
	"github.com/iii-hq/workers/mesh/internal/hub"
	"github.com/iii-hq/workers/mesh/internal/join"
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

func TestTokenInstallerArgumentsAndDigest(t *testing.T) {
	for _, mode := range []string{"valid", "digest"} {
		t.Run(mode, func(t *testing.T) {
			cmd, dir, _ := installerFixture(t, mode)
			pub, _, _ := ed25519.GenerateKey(rand.Reader)
			_, token, _ := join.NewToken("https://hub.example", pub, join.TokenOptions{})
			cmd.Args = []string{"sh", "install.sh", "0.1.0", "--token", token}
			out, err := cmd.CombinedOutput()
			b, readErr := os.ReadFile(filepath.Join(dir, "sudo-args"))
			if mode == "valid" {
				if err != nil {
					t.Fatalf("%s: %v", out, err)
				}
				if readErr != nil || !strings.HasSuffix(string(b), "join\n--token\n"+token+"\n") {
					t.Fatal("token arguments lost", readErr)
				}
			} else if err == nil || !os.IsNotExist(readErr) {
				t.Fatal("bad digest reached sudo")
			}
		})
	}
	for _, args := range [][]string{{"0.1.0", "--token"}, {"0.1.0", "--token", "bad"}, {"0.1.0", "--token", "iiimj_" + rand.Text()}, {"0.1.0", "--token", "bad", "https://h", "ABCD-EFGH"}, {"0.1.0", "https://h", "ABCD-EFGH", "extra"}} {
		cmd, dir, _ := installerFixture(t, "valid")
		cmd.Args = append([]string{"sh", "install.sh"}, args...)
		if err := cmd.Run(); err == nil {
			t.Fatal("accepted malformed options")
		}
		if _, err := os.Stat(filepath.Join(dir, "request")); !os.IsNotExist(err) {
			t.Fatal("invalid options reached registry")
		}
	}
}

func TestMain(m *testing.M) {
	if os.Getenv("MESH_INSTALLER_CLI_FIXTURE") == "1" {
		if len(os.Args) < 2 || os.Args[1] != "join" {
			os.Exit(2)
		}
		args, err := cli.ParseJoin(os.Args[2:])
		if err != nil {
			os.Exit(3)
		}
		b, _ := json.Marshal(args)
		if os.WriteFile(os.Getenv("MESH_INSTALLER_ARGS"), b, 0600) != nil {
			os.Exit(4)
		}
		os.Exit(0)
	}
	os.Exit(m.Run())
}
func executableInstallerFixture(t *testing.T, mode string) (*exec.Cmd, string, string) {
	t.Helper()
	cmd, dir, _ := installerFixture(t, "valid")
	binary, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	payload, err := os.ReadFile(binary)
	if err != nil {
		t.Fatal(err)
	}
	digest := sha256.Sum256(payload)
	sum := hex.EncodeToString(digest[:])
	if mode == "digest" {
		sum = strings.Repeat("0", 64)
	}
	entry := map[string]any{"name": "mesh", "version": "0.1.0", "type": "binary", "binaries": map[string]any{"x86_64-unknown-linux-gnu": map[string]string{"url": "https://artifacts.example/mesh", "sha256": sum}}}
	response, _ := json.Marshal(map[string]any{"graph": []any{entry}})
	if err = os.WriteFile(filepath.Join(dir, "response"), response, 0600); err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(filepath.Join(dir, "artifact"), payload, 0600); err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(filepath.Join(dir, "sudo"), []byte("#!/bin/sh\n[ \"$1\" = -- ] && shift\nexec \"$@\"\n"), 0700); err != nil {
		t.Fatal(err)
	}
	script, err := os.ReadFile("install.sh")
	if err != nil {
		t.Fatal(err)
	}
	os.WriteFile(filepath.Join(dir, "installer"), script, 0600)
	os.WriteFile(filepath.Join(dir, "curl"), []byte("#!/bin/sh\ncase \"$*\" in *raw.githubusercontent.com*) cat \"$FIXTURE/installer\";exit;; esac\nout=\nwhile [ $# -gt 0 ]; do\ncase \"$1\" in --output) shift;out=$1;; esac\nshift\ndone\nif [ -n \"$out\" ];then cp \"$FIXTURE/artifact\" \"$out\";else cat \"$FIXTURE/response\";fi\n"), 0700)
	pub, _, _ := ed25519.GenerateKey(rand.Reader)
	_, raw, err := join.NewToken("https://hub.example/prefix", pub, join.TokenOptions{})
	if err != nil {
		t.Fatal(err)
	}
	cmd.Env = append(cmd.Env, "MESH_INSTALLER_CLI_FIXTURE=1", "MESH_INSTALLER_ARGS="+filepath.Join(dir, "parsed"))
	return cmd, dir, raw
}
func TestReturnedUnixTokenLineRunsCLI(t *testing.T) {
	for _, mode := range []string{"valid", "digest"} {
		t.Run(mode, func(t *testing.T) {
			cmd, dir, raw := executableInstallerFixture(t, mode)
			line, _ := hub.TokenInstallLines("0.1.0", raw)
			cmd.Args = []string{"sh", "-c", line}
			out, err := cmd.CombinedOutput()
			b, readErr := os.ReadFile(filepath.Join(dir, "parsed"))
			if mode == "digest" {
				if err == nil || !os.IsNotExist(readErr) {
					t.Fatal("unverified binary ran")
				}
				return
			}
			if err != nil || readErr != nil {
				t.Fatalf("installer did not execute CLI: %v %v %s", err, readErr, out)
			}
			var parsed cli.JoinArgs
			if err = json.Unmarshal(b, &parsed); err != nil {
				t.Fatal(err)
			}
			if parsed.Token != raw || parsed.PublicURL != "https://hub.example/prefix" || parsed.Port != 41643 {
				t.Fatal("installer lost token/CLI semantics")
			}
		})
	}
}

func TestReturnedUnixTokenLinePreservesURLSchemeCase(t *testing.T) {
	for _, publicURL := range []string{"HTTPS://hub.example/CaseSensitive/Prefix", "hTtPs://hub.example/CaseSensitive/Prefix", "HtTp://hub.example/CaseSensitive/Prefix"} {
		t.Run(strings.Split(publicURL, ":")[0], func(t *testing.T) {
			cmd, dir, _ := executableInstallerFixture(t, "valid")
			pub, _, _ := ed25519.GenerateKey(rand.Reader)
			_, raw, err := join.NewToken(publicURL, pub, join.TokenOptions{})
			if err != nil {
				t.Fatal(err)
			}
			accepted, err := cli.ParseJoin([]string{"--token", raw})
			if err != nil || accepted.PublicURL != publicURL {
				t.Fatal("issuer/CLI rejected exact configured URL", err)
			}
			line, _ := hub.TokenInstallLines("0.1.0", raw)
			cmd.Args = []string{"sh", "-c", line}
			if out, err := cmd.CombinedOutput(); err != nil {
				t.Fatalf("returned installer rejected valid issued token: %v: %s", err, out)
			}
			b, err := os.ReadFile(filepath.Join(dir, "parsed"))
			if err != nil {
				t.Fatal(err)
			}
			var parsed cli.JoinArgs
			if err = json.Unmarshal(b, &parsed); err != nil {
				t.Fatal(err)
			}
			if parsed.Token != raw || parsed.PublicURL != publicURL {
				t.Fatal("installer rewrote token or exact URL")
			}
		})
	}
}
