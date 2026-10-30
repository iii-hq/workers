package coordinator

import (
	"tailscale.com/tailcfg"
	"testing"
)

func driveCoordinator(t *testing.T) *Coordinator {
	c := newCoordinator(t, t.TempDir())
	for _, name := range []string{"hub", "server", "laptop", "temporary", "windows", "other"} {
		registerNode(t, c, name)
		c.byName(name).OS = "linux"
		c.byName(name).Hostinfo = &tailcfg.Hostinfo{OS: "linux"}
	}
	c.byName("temporary").Ephemeral = true
	c.byName("windows").OS = "windows"
	return c
}
func TestDesiredExcludesEphemeralFromStar(t *testing.T) {
	c := driveCoordinator(t)
	if errs := c.SetDrives(map[string]Drive{"proj": {Server: "server", Nodes: []string{"*"}}}); len(errs) > 0 {
		t.Fatal(errs)
	}
	if got := c.Desired("temporary"); len(got.Mount) != 0 {
		t.Fatalf("ephemeral received %v", got)
	}
	if got := c.Desired("laptop"); len(got.Mount) != 1 || got.Mount[0].Addr != c.address(c.byName("server").ID) {
		t.Fatalf("missing authorized server address: %v", got)
	}
	if got := c.Desired("hub"); len(got.Mount) != 0 {
		t.Fatal("hub is not a mount-capable node")
	}
	c.SetDrives(map[string]Drive{"proj": {Server: "server", Nodes: []string{"temporary"}}})
	if len(c.Desired("temporary").Mount) != 1 {
		t.Fatal("explicit ephemeral membership lost")
	}
}
func TestServerMustBeLinux(t *testing.T) {
	c := driveCoordinator(t)
	for _, server := range []string{"missing", "windows", "hub"} {
		if errs := c.SetDrives(map[string]Drive{"proj": {Server: server, Nodes: []string{"*"}}}); len(errs) != 1 {
			t.Fatalf("accepted %s: %v", server, errs)
		}
		if len(c.Desired("laptop").Mount) != 0 {
			t.Fatal("invalid server survived replacement")
		}
	}
}
func TestExportsOnlyMembers(t *testing.T) {
	c := driveCoordinator(t)
	c.SetDrives(map[string]Drive{"proj": {Server: "server", Nodes: []string{"laptop", "windows"}}})
	d := c.Desired("server")
	if d.Prefix != c.Prefix() || len(d.Serve) != 1 || len(d.Serve[0].Clients) != 2 {
		t.Fatalf("bad export %+v", d)
	}
	for _, client := range d.Serve[0].Clients {
		if client.Name != "laptop" && client.Name != "windows" {
			t.Fatal(client.Name)
		}
		if client.Addr != c.address(c.byName(client.Name).ID) {
			t.Fatal("wrong client address")
		}
		if client.Name == "windows" {
			m := c.Desired("windows").Mount[0]
			if client.SMB == nil || m.SMB == nil || *m.SMB != *client.SMB {
				t.Fatal("credential mismatch")
			}
		} else if client.SMB != nil {
			t.Fatal("NFS client received credential")
		}
	}
	if !c.sees(c.byName("server"), c.byName("laptop")) || c.sees(c.byName("laptop"), c.byName("windows")) {
		t.Fatal("incorrect drive topology")
	}
	c.byName("laptop").Removed = true
	if len(c.Desired("server").Serve[0].Clients) != 1 || len(c.Desired("laptop").Mount) != 0 {
		t.Fatal("removed member retained access")
	}
	c.SetDrives(nil)
	if len(c.Desired("server").Serve) != 0 || c.sees(c.byName("server"), c.byName("windows")) {
		t.Fatal("empty replacement retained drive")
	}
}

func TestAuthenticatedDriveServerMetadataWakesPeerMaps(t *testing.T) {
	c := driveCoordinator(t)
	c.byName("server").OS = ""
	c.SetDrives(map[string]Drive{"proj": {Server: "server", Nodes: []string{"laptop"}}})
	wake := make(chan struct{}, 1)
	c.wakes[wake] = true
	if err := c.SetRuntimeMetadata("server", RuntimeMetadata{OS: "linux", Arch: "amd64", Version: "1.0.0"}); err != nil {
		t.Fatal(err)
	}
	select {
	case <-wake:
	default:
		t.Fatal("authenticated server metadata did not wake waiting member maps")
	}
}
