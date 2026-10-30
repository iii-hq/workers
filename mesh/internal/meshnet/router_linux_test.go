package meshnet

import (
	"net/netip"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"tailscale.com/wgengine/router"
)

func fakeIP(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	log := filepath.Join(dir, "commands")
	script := `#!/bin/sh
printf '%s\n' "$*" >> "$III_MESH_TEST_COMMANDS"
case "$*" in
  '-6 rule del table 5200')
    if [ "$III_MESH_TEST_RULE_ERROR" = permission ]; then
      printf '%s\n' 'RTNETLINK answers: Operation not permitted' >&2
      exit 2
    fi
    printf '%s\n' 'RTNETLINK answers: No such file or directory' >&2
    exit 2;;
esac
if [ "$III_MESH_TEST_FAIL_ROUTE" = 1 ]; then
  case "$*" in '-6 route replace '*) exit 1;; esac
fi
`
	if err := os.WriteFile(filepath.Join(dir, "ip"), []byte(script), 0700); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", dir)
	t.Setenv("III_MESH_TEST_COMMANDS", log)
	return log
}

func TestLinuxRouterConfiguresOnlyMeshPrefix(t *testing.T) {
	commands := fakeIP(t)
	r := &osRouter{tun: "iii0"}
	if err := r.Up(); err != nil {
		t.Fatal(err)
	}
	config := &router.Config{LocalAddrs: []netip.Prefix{netip.MustParsePrefix("fd7a:1234:5678::2/128")}, Routes: []netip.Prefix{netip.MustParsePrefix("::/0")}}
	if err := r.Set(config); err != nil {
		t.Fatal(err)
	}
	if err := r.Close(); err != nil {
		t.Fatal(err)
	}
	b, err := os.ReadFile(commands)
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{"link set dev iii0 up", "-6 addr replace fd7a:1234:5678::2/128 dev iii0 nodad", "-6 route replace fd7a:1234:5678::/48 dev iii0 table 5200", "-6 rule add to fd7a:1234:5678::/48 table 5200 priority 5200", "-6 addr del fd7a:1234:5678::2/128 dev iii0"} {
		if !strings.Contains(string(b), want+"\n") {
			t.Errorf("missing command %q in %s", want, b)
		}
	}
	if strings.Contains(string(b), "::/0") {
		t.Fatal("installed unrelated route")
	}
}

func TestLinuxRouterRollsBackPartialConfiguration(t *testing.T) {
	commands := fakeIP(t)
	t.Setenv("III_MESH_TEST_FAIL_ROUTE", "1")
	r := &osRouter{tun: "iii0"}
	if err := r.Set(&router.Config{LocalAddrs: []netip.Prefix{netip.MustParsePrefix("fd7a:1234:5678::2/128")}}); err == nil {
		t.Fatal("ignored routing failure")
	}
	b, err := os.ReadFile(commands)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(b), "-6 addr del fd7a:1234:5678::2/128 dev iii0\n") {
		t.Fatalf("partial address not removed: %s", b)
	}
}

func TestLinuxRouterReportsRuleDeletionErrors(t *testing.T) {
	for _, failure := range []string{"permission", "executable"} {
		t.Run(failure, func(t *testing.T) {
			commands := fakeIP(t)
			if failure == "permission" {
				t.Setenv("III_MESH_TEST_RULE_ERROR", "permission")
			} else {
				if err := os.Remove(filepath.Join(filepath.Dir(commands), "ip")); err != nil {
					t.Fatal(err)
				}
			}
			r := &osRouter{tun: "iii0"}
			err := r.Set(&router.Config{LocalAddrs: []netip.Prefix{netip.MustParsePrefix("fd7a:1234:5678::2/128")}})
			if err == nil {
				t.Fatal("ignored rule deletion failure during setup")
			}
			if !strings.Contains(err.Error(), "rule del table 5200") {
				t.Fatalf("rule deletion error lost: %v", err)
			}
			if failure == "permission" {
				if !strings.Contains(err.Error(), "Operation not permitted") {
					t.Fatalf("permission error lost: %v", err)
				}
				b, readErr := os.ReadFile(commands)
				if readErr != nil {
					t.Fatal(readErr)
				}
				if strings.Contains(string(b), "addr replace") {
					t.Fatalf("configured address despite stale-rule failure: %s", b)
				}
			}
		})
	}
}

func TestLinuxRouterCleanupReportsRuleDeletionErrorAndContinues(t *testing.T) {
	commands := fakeIP(t)
	r := &osRouter{tun: "iii0"}
	if err := r.Set(&router.Config{LocalAddrs: []netip.Prefix{netip.MustParsePrefix("fd7a:1234:5678::2/128")}}); err != nil {
		t.Fatal(err)
	}
	t.Setenv("III_MESH_TEST_RULE_ERROR", "permission")
	err := r.Close()
	if err == nil || !strings.Contains(err.Error(), "rule del table 5200") || !strings.Contains(err.Error(), "Operation not permitted") {
		t.Fatalf("cleanup discarded rule deletion error: %v", err)
	}
	b, err := os.ReadFile(commands)
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{"-6 route flush table 5200\n", "-6 addr del fd7a:1234:5678::2/128 dev iii0\n"} {
		if !strings.Contains(string(b), want) {
			t.Errorf("cleanup skipped %q: %s", want, b)
		}
	}
}
