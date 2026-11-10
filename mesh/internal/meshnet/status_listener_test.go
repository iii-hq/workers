//go:build linux || darwin

package meshnet

import (
	"net"
	"os"
	"path/filepath"
	"testing"
)

func TestRemovedStatusListenerOwnership(t *testing.T) {
	// t.TempDir() embeds the test name under $TMPDIR and overflows the 104-byte
	// unix socket path limit on macOS.
	dir, err := os.MkdirTemp("/tmp", "mesh-status-")
	if err != nil {
		t.Fatal(err)
	}
	old := socket
	socket = filepath.Join(dir, "status.sock")
	t.Cleanup(func() { socket = old; os.RemoveAll(dir) })
	server, err := ServeStatus(func() string { return "removed from the mesh by its hub\nHub: https://hub.example\n" })
	if err != nil {
		t.Fatal(err)
	}
	defer server.Close()
	text, err := Status()
	if err != nil || text != "removed from the mesh by its hub\nHub: https://hub.example\n" {
		t.Fatal(text, err)
	}
	st, err := os.Stat(socket)
	if err != nil || st.Mode().Perm()&0006 != 0006 {
		t.Fatal("operator cannot access socket", st, err)
	}
	if second, err := ServeStatus(func() string { return "takeover" }); err == nil {
		second.Close()
		t.Fatal("replaced live socket")
	}
	text, err = ask("leave")
	if err != nil || text != "" {
		t.Fatal("accepted mutable command", text, err)
	}
	if err := server.Close(); err != nil {
		t.Fatal(err)
	}
	if c, err := net.Dial("unix", socket); err == nil {
		c.Close()
		t.Fatal("listener remains")
	}
}
