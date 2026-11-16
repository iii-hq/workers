package main

import (
	"context"
	"io/fs"
	"maps"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"tailscale.com/feature/buildfeatures"
)

// The worker reacts to events: the only periodic timer this code arms itself is the map keepalive, a heartbeat the tailscale protocol requires; the netstack TCP keepalive and the HTTP/2 idle ping are heartbeats configured on the libraries.
func TestNoPeriodicTimers(t *testing.T) {
	found := map[string]int{}
	for _, root := range []string{"main.go", "internal", "ui"} {
		err := filepath.WalkDir(root, func(path string, d fs.DirEntry, err error) error {
			if err != nil || d.IsDir() {
				if d != nil && d.Name() == "node_modules" {
					return filepath.SkipDir
				}
				return err
			}
			if ext := filepath.Ext(path); strings.HasSuffix(path, "_test.go") || ext != ".go" && ext != ".ts" && ext != ".tsx" {
				return nil
			}
			b, err := os.ReadFile(path)
			for _, form := range []string{"NewTicker(", "time.Tick(", "time.Sleep(", "time.After(", "setInterval("} {
				if n := strings.Count(string(b), form); n > 0 {
					found[path+" "+form] = n
				}
			}
			return err
		})
		if err != nil {
			t.Fatal(err)
		}
	}
	if want := map[string]int{filepath.Join("internal", "coordinator", "coordinator.go") + " NewTicker(": 1}; !maps.Equal(found, want) {
		t.Fatalf("timers found %v, want only %v: replace a periodic loop with the event it waits for", found, want)
	}
}

// The peer API server listens on the host outside the tunnel and trusts a source address a LAN attacker can forge.
func TestBuiltWithoutPeerAPIServer(t *testing.T) {
	if buildfeatures.HasPeerAPIServer {
		t.Fatal("built with tailscale's peer API server: build and test with -tags ts_omit_peerapiserver (make build, make test)")
	}
}

func TestRetiredCommandsAreRefused(t *testing.T) {
	for _, args := range [][]string{{"join", "https://h", "ABCD-EFGH"}, {"join", "--token", "x"}, {"leave"}, {"node"}, {"status"}} {
		if err := run(context.Background(), args); err == nil || !strings.Contains(err.Error(), "unexpected positional arguments") {
			t.Fatalf("%v: %v", args, err)
		}
	}
}
