package coordinator

import (
	"os"
	"path/filepath"
	"testing"
)

func TestRuntimeMetadataPersistsWithoutChangingActivity(t *testing.T) {
	dir := t.TempDir()
	c := newCoordinator(t, dir)
	registerNode(t, c, "laptop")
	before := c.Nodes()[0]
	if err := c.SetRuntimeMetadata("laptop", RuntimeMetadata{OS: "linux", Arch: "amd64", Version: "0.2.0", Target: "x86_64-unknown-linux-musl"}); err != nil {
		t.Fatal(err)
	}
	if err := c.SetOperator("laptop", "alice"); err != nil {
		t.Fatal(err)
	}
	c.Close()
	restored := newCoordinator(t, dir)
	got := restored.Nodes()[0]
	if got.OS != "linux" || got.Arch != "amd64" || got.Version != "0.2.0" || got.Target != "x86_64-unknown-linux-musl" || got.Operator != "alice" {
		t.Fatalf("metadata %+v", got)
	}
	if got.JoinedAt != before.JoinedAt || got.LastSeen != before.LastSeen {
		t.Fatal("metadata changed registration/activity timestamps")
	}
	for _, m := range []RuntimeMetadata{{OS: "linux", Arch: "arm64", Version: "v", Target: "x86_64-unknown-linux-gnu"}, {OS: "linux", Arch: "amd64", Version: "", Target: "x86_64-unknown-linux-gnu"}} {
		if err := restored.SetRuntimeMetadata("laptop", m); err == nil {
			t.Fatal("invalid runtime metadata accepted")
		}
	}
}
func TestRuntimeMetadataWriteFailureRollsBack(t *testing.T) {
	c := newCoordinator(t, t.TempDir())
	registerNode(t, c, "laptop")
	blocker := filepath.Join(t.TempDir(), "file")
	os.WriteFile(blocker, nil, 0600)
	c.file = filepath.Join(blocker, "state.json")
	if err := c.SetRuntimeMetadata("laptop", RuntimeMetadata{OS: "linux", Arch: "amd64", Version: "v", Target: "x86_64-unknown-linux-gnu"}); err == nil {
		t.Fatal("write succeeded")
	}
	if c.Nodes()[0].Version != "" {
		t.Fatal("failed save leaked metadata")
	}
}
