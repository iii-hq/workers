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
  '-6 rule del table 5200') exit 1;;
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
