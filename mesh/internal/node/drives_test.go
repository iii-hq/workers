package node

import (
	"context"
	"encoding/json"
	"io/fs"
	"net/netip"
	"testing"
	"time"

	"github.com/iii-hq/workers/mesh/internal/drives"
)

type mountHost struct {
	drives.Host
	mounted map[string]string
	calls   int
}

func (h *mountHost) Run(_ context.Context, c drives.Command) (string, error) {
	h.calls++
	if c.Program == "mount" {
		h.mounted[c.Args[len(c.Args)-1]] = c.Args[len(c.Args)-2]
	}
	if c.Program == "umount" {
		delete(h.mounted, c.Args[len(c.Args)-1])
	}
	return "", nil
}
func (h *mountHost) MkdirAll(string, fs.FileMode) error                { return nil }
func (h *mountHost) Remove(string) error                               { return nil }
func (h *mountHost) LookPath(s string) (string, error)                 { return "/usr/bin/" + s, nil }
func (h *mountHost) Mounts(context.Context) (map[string]string, error) { return h.mounted, nil }
func (h *mountHost) Reach(context.Context, netip.Addr, string) bool    { return true }
func TestSignedDrivesApplyUsesManagerAndCompleteReplacement(t *testing.T) {
	n, key, _, _ := testNode(t)
	host := &mountHost{mounted: map[string]string{}}
	m, err := drives.NewWithHost(drives.Options{StateDir: t.TempDir(), Platform: "linux", Operator: "alice", UID: 1000, GID: 1000}, host)
	if err != nil {
		t.Fatal(err)
	}
	defer m.Close()
	n.drives = NewDriveController(m)
	desired := drives.Desired{Prefix: netip.MustParsePrefix("fd10:20:30::/48"), Mount: []drives.Mount{{Name: "proj", Server: "server", Host: "server.iii.internal", Addr: netip.MustParseAddr("fd10:20:30::1")}}}
	request, _ := Sign(key, "mesh-node::drives::apply", desired, time.Now())
	bad := request
	bad.Sig = "invalid"
	if _, err = n.handle(t.Context(), "mesh-node::drives::apply", bad); err == nil || host.calls != 0 {
		t.Fatal("unsigned mount escaped verifier")
	}
	raw, err := n.handle(t.Context(), "mesh-node::drives::apply", request)
	if err != nil {
		t.Fatal(err)
	}
	var result []DriveStatus
	json.Unmarshal(raw, &result)
	if len(result) != 1 || result[0].State != "mounted" || host.mounted["/drives/proj"] != "[fd10:20:30::1]:/srv/iii-mesh/proj" || len(n.Status().Drives) != 1 {
		t.Fatalf("lost signed desired: %s %+v", raw, n.Status())
	}
	request, _ = Sign(key, "mesh-node::drives::apply", drives.Desired{}, time.Now())
	if _, err = n.handle(t.Context(), "mesh-node::drives::apply", request); err != nil {
		t.Fatal(err)
	}
	if len(host.mounted) != 0 || len(n.Status().Drives) != 0 {
		t.Fatal("empty signed replacement retained mount")
	}
}
