package hub

import (
	"context"
	"crypto/rand"
	"encoding/json"
	"errors"
	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/meshnet"
	"io/fs"
	"net/http"
	"net/http/httptest"
	"net/netip"
	"path/filepath"
	"strings"
	"tailscale.com/tsnet"
	"testing"
	"time"

	"github.com/iii-hq/workers/mesh/internal/drives"
	"github.com/iii-hq/workers/mesh/internal/node"
)

type hubDriveHost struct {
	files       map[string][]byte
	mounts      map[string]string
	commands    []drives.Command
	fail        string
	occupied    string
	unreachable bool
}

func newHubDriveHost() *hubDriveHost {
	return &hubDriveHost{files: map[string][]byte{}, mounts: map[string]string{}}
}
func (h *hubDriveHost) Run(_ context.Context, c drives.Command) (string, error) {
	h.commands = append(h.commands, c)
	if c.Program == h.fail {
		return "", errors.New("injected " + c.Program + " failure")
	}
	if c.Program == "useradd" {
		u := c.Args[len(c.Args)-1]
		h.files["/etc/passwd"] = append(h.files["/etc/passwd"], []byte(u+":x:1001:1001:iii mesh drive client:/nonexistent:/usr/sbin/nologin\n")...)
	}
	if c.Program == "userdel" {
		var keep []string
		for line := range strings.Lines(string(h.files["/etc/passwd"])) {
			if !strings.HasPrefix(line, c.Args[0]+":") {
				keep = append(keep, line)
			}
		}
		h.files["/etc/passwd"] = []byte(strings.Join(keep, ""))
	}
	if c.Program == "ss" {
		return h.occupied, nil
	}
	if c.Program == "exportfs" && len(c.Args) == 0 {
		return "", nil
	}
	if c.Program == "mount" {
		h.mounts[c.Args[len(c.Args)-1]] = c.Args[len(c.Args)-2]
	}
	if c.Program == "umount" {
		delete(h.mounts, c.Args[len(c.Args)-1])
	}
	if c.Program == "net" && len(c.Args) > 2 {
		if c.Args[2] == "/delete" {
			delete(h.mounts, c.Args[1])
		} else {
			h.mounts[c.Args[1]] = c.Args[2]
		}
	}
	return "", nil
}
func (h *hubDriveHost) ReadFile(p string) ([]byte, error) {
	b, ok := h.files[p]
	if !ok {
		return nil, fs.ErrNotExist
	}
	return b, nil
}
func (h *hubDriveHost) WriteFile(p string, b []byte, _ fs.FileMode) error {
	h.files[p] = append([]byte{}, b...)
	return nil
}
func (h *hubDriveHost) MkdirAll(string, fs.FileMode) error { return nil }
func (h *hubDriveHost) Remove(p string) error              { delete(h.files, p); return nil }
func (h *hubDriveHost) RemoveAll(p string) error {
	for path := range h.files {
		if path == p || strings.HasPrefix(path, p+"/") {
			delete(h.files, path)
		}
	}
	return nil
}
func (h *hubDriveHost) Glob(pattern string) ([]string, error) {
	var names []string
	for p := range h.files {
		if ok, _ := filepath.Match(pattern, p); ok {
			names = append(names, p)
		}
	}
	return names, nil
}
func (h *hubDriveHost) Chown(string, int, int) error         { return nil }
func (h *hubDriveHost) LookPath(name string) (string, error) { return "/usr/sbin/" + name, nil }
func (h *hubDriveHost) Mounts(context.Context) (map[string]string, error) {
	m := map[string]string{}
	for k, v := range h.mounts {
		m[k] = v
	}
	return m, nil
}
func (h *hubDriveHost) Reach(context.Context, netip.Addr, string) bool { return !h.unreachable }

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
