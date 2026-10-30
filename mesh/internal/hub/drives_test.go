package hub

import (
	"context"
	"crypto/rand"
	"encoding/json"
	"errors"
	"fmt"
	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/meshnet"
	"io/fs"
	"maps"
	"net/http"
	"net/http/httptest"
	"net/netip"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"tailscale.com/tsnet"
	"testing"
	"time"

	"github.com/iii-hq/workers/mesh/internal/drives"
	"github.com/iii-hq/workers/mesh/internal/node"
)

type hubDriveHost struct {
	root   string
	mounts map[string]string
	fail   string
}

func newHubDriveHost(t *testing.T) *hubDriveHost {
	return &hubDriveHost{root: t.TempDir(), mounts: map[string]string{}}
}
func (h *hubDriveHost) path(p string) string {
	return filepath.Join(h.root, filepath.FromSlash(strings.TrimPrefix(p, "/")))
}
func (h *hubDriveHost) Run(_ context.Context, c drives.Command) (string, error) {
	if c.Program == h.fail {
		return "", errors.New("injected " + c.Program + " failure")
	}
	if c.Operator || c.Sensitive || c.Input != "" {
		return "", errors.New("self-serve fixture received an unexpected operator/credential command")
	}
	switch c.Program {
	case "mount":
		if len(c.Args) == 4 && c.Args[0] == "-o" && c.Args[1] == "bind" && strings.HasPrefix(c.Args[2], "/srv/iii-mesh/") && strings.HasPrefix(c.Args[3], "/drives/") {
			h.mounts[c.Args[3]] = c.Args[2]
			return "", nil
		}
	case "umount":
		if len(c.Args) == 3 && reflect.DeepEqual(c.Args[:2], []string{"-i", "-c"}) {
			delete(h.mounts, c.Args[2])
			return "", nil
		}
	case "exportfs":
		if len(c.Args) == 0 || reflect.DeepEqual(c.Args, []string{"-ra"}) {
			return "", nil
		}
	case "ss":
		if reflect.DeepEqual(c.Args, []string{"-Hltnp", "sport", "=", ":445"}) {
			return "", nil
		}
	case "systemctl":
		if reflect.DeepEqual(c.Args, []string{"daemon-reload"}) {
			return "", nil
		}
		if len(c.Args) == 2 && (c.Args[1] == "iii-mesh-smbd" || c.Args[1] == "nfs-server") && (c.Args[0] == "start" || c.Args[0] == "stop" || c.Args[0] == "restart") {
			return "", nil
		}
	}
	return "", fmt.Errorf("unexpected self-serve command: %s %v", c.Program, c.Args)
}
func (h *hubDriveHost) ReadFile(p string) ([]byte, error) { return os.ReadFile(h.path(p)) }
func (h *hubDriveHost) WriteFile(p string, b []byte, mode fs.FileMode) error {
	return os.WriteFile(h.path(p), b, mode)
}

// Model the privileged filesystem using a writable, unprivileged sandbox.
func (h *hubDriveHost) MkdirAll(p string, _ fs.FileMode) error { return os.MkdirAll(h.path(p), 0700) }
func (h *hubDriveHost) Remove(p string) error                  { return os.Remove(h.path(p)) }
func (h *hubDriveHost) RemoveAll(p string) error               { return os.RemoveAll(h.path(p)) }
func (h *hubDriveHost) Glob(pattern string) ([]string, error) {
	paths, err := filepath.Glob(h.path(pattern))
	for i, p := range paths {
		paths[i] = "/" + filepath.ToSlash(strings.TrimPrefix(p, h.root+string(filepath.Separator)))
	}
	return paths, err
}
func (h *hubDriveHost) Chown(string, int, int) error         { return nil }
func (h *hubDriveHost) LookPath(name string) (string, error) { return "/usr/sbin/" + name, nil }
func (h *hubDriveHost) Mounts(context.Context) (map[string]string, error) {
	return maps.Clone(h.mounts), nil
}
func (h *hubDriveHost) Reach(context.Context, netip.Addr, string) bool {
	panic("self-served bind mounts must not probe a remote server")
}

func realDriveController(t *testing.T, host *hubDriveHost) node.DriveController {
	t.Helper()
	manager, err := drives.NewWithHost(drives.Options{StateDir: t.TempDir(), Platform: "linux", Operator: "alice", UID: 1000, GID: 1000}, host)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { manager.Close() })
	return node.NewDriveController(manager)
}

type driveWatchRPC struct {
	rpc
	updates chan struct{}
}

func (r driveWatchRPC) Trigger(ctx context.Context, req iii.TriggerRequest) (json.RawMessage, error) {
	raw, err := r.rpc.Trigger(ctx, req)
	if strings.HasSuffix(req.FunctionID, "::drives::apply") {
		select {
		case r.updates <- struct{}{}:
		default:
		}
	}
	return raw, err
}
func TestDriveMemberRemovalReconcilesExistingServer(t *testing.T) {
	h, r := nodeCallHub(t)
	r.status.Version = h.Version
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	updates := make(chan struct{}, 32)
	h.nodeClient = driveWatchRPC{rpc: r, updates: updates}
	h.startNodeCalls(ctx)
	defer func() { h.stopNodeCalls(); <-h.nodeReconciler.done }()
	select {
	case <-updates:
	case <-time.After(5 * time.Second):
		t.Fatal("initial reconcile missing")
	}
	// The watcher must observe membership even when no connected node changed.
	control := http.NewServeMux()
	h.Coordinator.Handle(control)
	front := httptest.NewServer(control)
	defer front.Close()
	shim, err := meshnet.NewShim(front.URL, h.Coordinator.Public())
	if err != nil {
		t.Fatal(err)
	}
	defer shim.Close()
	peer := &tsnet.Server{Dir: t.TempDir(), Hostname: "member", ControlURL: shim.URL(), AuthKey: rand.Text(), Logf: func(string, ...any) {}, UserLogf: func(string, ...any) {}}
	defer peer.Close()
	joined, stop := context.WithTimeout(ctx, 10*time.Second)
	defer stop()
	if _, err = peer.Up(joined); err != nil {
		t.Fatal(err)
	}
	deadline := time.After(3 * time.Second)
	for {
		h.nodeReconciler.mu.Lock()
		observed := h.nodeReconciler.tasks["member"] != nil && h.nodeReconciler.tasks["laptop"] == nil
		h.nodeReconciler.mu.Unlock()
		if observed {
			break
		}
		select {
		case <-deadline:
			t.Fatal("new member not observed")
		case <-time.After(time.Millisecond):
		}
	}
	for len(updates) > 0 {
		<-updates
	}
	if err = h.Coordinator.Remove("member"); err != nil {
		t.Fatal(err)
	}
	select {
	case <-updates:
	case <-time.After(3 * time.Second):
		t.Fatal("member removal did not reconcile existing server")
	}
}
