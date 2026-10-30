package drives

import (
	"context"
	"encoding/json"
	"errors"
	"github.com/iii-hq/workers/mesh/internal/supervisor"
	"io/fs"
	"maps"
	"net/netip"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"
)

type testHost struct {
	files       map[string][]byte
	mounts      map[string]string
	commands    []Command
	fail        string
	occupied    string
	unreachable bool
}

func hostFixture() *testHost {
	return &testHost{files: map[string][]byte{}, mounts: map[string]string{}}
}
func (h *testHost) Run(_ context.Context, c Command) (string, error) {
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
func (h *testHost) ReadFile(p string) ([]byte, error) {
	b, ok := h.files[p]
	if !ok {
		return nil, fs.ErrNotExist
	}
	return b, nil
}
func (h *testHost) WriteFile(p string, b []byte, _ fs.FileMode) error {
	h.files[p] = append([]byte{}, b...)
	return nil
}
func (h *testHost) MkdirAll(string, fs.FileMode) error { return nil }
func (h *testHost) Remove(p string) error              { delete(h.files, p); return nil }
func (h *testHost) RemoveAll(p string) error {
	for path := range h.files {
		if path == p || strings.HasPrefix(path, p+"/") {
			delete(h.files, path)
		}
	}
	return nil
}
func (h *testHost) Glob(pattern string) ([]string, error) {
	var names []string
	for p := range h.files {
		if ok, _ := filepath.Match(pattern, p); ok {
			names = append(names, p)
		}
	}
	return names, nil
}
func (h *testHost) Chown(string, int, int) error         { return nil }
func (h *testHost) LookPath(name string) (string, error) { return "/usr/sbin/" + name, nil }
func (h *testHost) Mounts(context.Context) (map[string]string, error) {
	m := map[string]string{}
	for k, v := range h.mounts {
		m[k] = v
	}
	return m, nil
}
func (h *testHost) Reach(context.Context, netip.Addr, string) bool { return !h.unreachable }
func fixtureDesired() Desired {
	return Desired{Prefix: netip.MustParsePrefix("fd31:32:33::/48"), Mount: []Mount{{Name: "proj", Server: "server", Host: "server.iii.internal", Addr: netip.MustParseAddr("fd31:32:33::1")}}}
}
func managerFixture(t *testing.T, platform, dir string, h Host) *Manager {
	t.Helper()
	m, err := NewWithHost(Options{StateDir: dir, Platform: platform, Operator: "alice", UID: 1000, GID: 1000}, h)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { m.Close() })
	return m
}
func requireOK(t *testing.T, r []Result) {
	t.Helper()
	for _, d := range r {
		if d.Error != "" {
			t.Fatalf("unexpected drive failure: %+v", d)
		}
	}
}
func TestMountTargets(t *testing.T) {
	for _, tc := range []struct{ platform, path, source string }{{"linux", "/drives/proj", "[fd31:32:33::1]:/srv/iii-mesh/proj"}, {"darwin", "/Volumes/proj", "server.iii.internal:/srv/iii-mesh/proj"}, {"windows", "W:", `\\server.iii.internal\proj`}} {
		t.Run(tc.platform, func(t *testing.T) {
			h := hostFixture()
			m := managerFixture(t, tc.platform, t.TempDir(), h)
			d := fixtureDesired()
			d.Mount[0].SMB = &SMB{User: "iii-client", Password: strings.Repeat("a", 64)}
			r := m.Apply(t.Context(), d)
			requireOK(t, r)
			if len(r) != 1 || r[0].Path != tc.path || h.mounts[tc.path] != tc.source {
				t.Fatalf("results %+v mounts %v", r, h.mounts)
			}
			if tc.platform == "windows" {
				for _, c := range h.commands {
					if !c.Operator {
						t.Fatal("Windows command escaped operator session")
					}
				}
			} else {
				var opts string
				for _, c := range h.commands {
					if c.Program == "mount" {
						opts = strings.Join(c.Args, " ")
					}
				}
				if !strings.Contains(opts, "hard") || tc.platform == "darwin" && !strings.Contains(opts, "vers=4.0") {
					t.Fatal(opts)
				}
			}
		})
	}
}
func TestUnknownDesiredAndRestartPreserveMounts(t *testing.T) {
	h := hostFixture()
	dir := t.TempDir()
	m := managerFixture(t, "linux", dir, h)
	requireOK(t, m.Apply(t.Context(), fixtureDesired()))
	before := len(h.commands)
	m.Close()
	again := managerFixture(t, "linux", dir, h)
	if len(h.commands) != before || len(h.mounts) != 1 {
		t.Fatal("restart changed mounts")
	}
	requireOK(t, again.Apply(t.Context(), Desired{}))
	if len(h.mounts) != 0 {
		t.Fatal("empty replacement kept mount")
	}
}
func TestCleanupFailureRetainsJournalForRetry(t *testing.T) {
	h := hostFixture()
	dir := t.TempDir()
	m := managerFixture(t, "linux", dir, h)
	requireOK(t, m.Apply(t.Context(), fixtureDesired()))
	h.fail = "umount"
	if err := m.Drop(t.Context()); err == nil {
		t.Fatal("cleanup failure swallowed")
	}
	m.Close()
	h.fail = ""
	again := managerFixture(t, "linux", dir, h)
	if err := again.Drop(t.Context()); err != nil {
		t.Fatal(err)
	}
	if len(h.mounts) != 0 {
		t.Fatal("retry lost owned mount")
	}
}
func TestSmbdUnitConfinedToMesh(t *testing.T) {
	h := hostFixture()
	m := managerFixture(t, "linux", t.TempDir(), h)
	d := fixtureDesired()
	d.Mount = nil
	d.Serve = []Export{{Name: "proj", Clients: []Client{{Name: "win", Addr: netip.MustParseAddr("fd31:32:33::9"), SMB: &SMB{User: "iii-client", Password: strings.Repeat("b", 64)}}}}}
	requireOK(t, m.Apply(t.Context(), d))
	unit := string(h.files["/etc/systemd/system/iii-mesh-smbd.service"])
	if !strings.Contains(unit, "IPAddressDeny=any\n") || !strings.Contains(unit, "IPAddressAllow=fd31:32:33::/48\n") {
		t.Fatal(unit)
	}
	exports := string(h.files["/etc/exports.d/iii-mesh.exports"])
	if !strings.Contains(exports, "/srv/iii-mesh/proj fd31:32:33::9(rw,sync,all_squash,anonuid=1000,anongid=1000,no_subtree_check)") || strings.Contains(exports, "*(") {
		t.Fatal(exports)
	}
	requireOK(t, m.Apply(t.Context(), Desired{}))
	if len(h.files["/etc/exports.d/iii-mesh.exports"]) > 0 || len(h.files["/etc/systemd/system/iii-mesh-smbd.service"]) > 0 {
		t.Fatal("server resources not removed")
	}
}
func TestServerPreflightRefusesExistingResources(t *testing.T) {
	for _, kind := range []string{"exports", "live exports", "port"} {
		t.Run(kind, func(t *testing.T) {
			h := hostFixture()
			if kind == "exports" {
				h.files["/etc/exports"] = []byte("/existing *(rw)\n")
			}
			if kind == "port" {
				h.occupied = "listener"
			}
			if kind == "live exports" {
				h.fail = "exportfs"
			}
			m := managerFixture(t, "linux", t.TempDir(), h)
			d := fixtureDesired()
			d.Mount = nil
			d.Serve = []Export{{Name: "proj"}}
			r := m.Apply(t.Context(), d)
			if len(r) != 1 || r[0].Error == "" {
				t.Fatal(r)
			}
			if len(h.files) != 0 && kind != "exports" {
				t.Fatal("refusal changed files")
			}
			for _, c := range h.commands {
				if c.Program == "apt-get" || c.Program == "systemctl" {
					t.Fatalf("refusal mutated services %+v", c)
				}
			}
		})
	}
}
func TestJournalRejectsArbitraryTargets(t *testing.T) {
	h := hostFixture()
	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, "drives.json"), []byte(`{"mounts":{"proj":{"name":"proj","target":"/etc","source":"evil"}}}`), 0600)
	if _, err := NewWithHost(Options{StateDir: dir, Platform: "linux", Operator: "alice", UID: 1000, GID: 1000}, h); err == nil {
		t.Fatal("accepted forged path")
	}
	if len(h.commands) != 0 {
		t.Fatal("invalid journal ran commands")
	}
}
func TestMountUnreachableAndForeignTargetPreserved(t *testing.T) {
	h := hostFixture()
	m := managerFixture(t, "linux", t.TempDir(), h)
	requireOK(t, m.Apply(t.Context(), fixtureDesired()))
	h.unreachable = true
	r := m.Apply(t.Context(), fixtureDesired())
	if r[0].State != "waiting" || len(h.mounts) != 1 {
		t.Fatal(r)
	}
	h.mounts["/drives/proj"] = "foreign"
	before := len(h.commands)
	r = m.Apply(t.Context(), Desired{})
	if len(r) != 1 || r[0].State != "error" || len(h.commands) != before || !reflect.DeepEqual(h.mounts, map[string]string{"/drives/proj": "foreign"}) {
		t.Fatalf("foreign mount touched: %+v %v", r, h.commands)
	}
}

func TestNativeCommandRedactsSensitiveFailure(t *testing.T) {
	h := &nativeHost{}
	secret := strings.Repeat("c", 64)
	out, err := h.Run(t.Context(), Command{Program: "sh", Args: []string{"-c", "printf '%s' \"$1\"; exit 1", "sh", secret}, Sensitive: true})
	if err == nil || strings.Contains(err.Error(), secret) || strings.Contains(out, secret) {
		t.Fatalf("sensitive command leaked output %q %v", out, err)
	}
}

func TestServerRevokesAccessBeforeFailingMembershipUpdate(t *testing.T) {
	h := hostFixture()
	m := managerFixture(t, "linux", t.TempDir(), h)
	d := fixtureDesired()
	d.Mount = nil
	d.Serve = []Export{{Name: "proj", Clients: []Client{{Name: "win", Addr: netip.MustParseAddr("fd31:32:33::9"), SMB: &SMB{User: "iii-client", Password: strings.Repeat("d", 64)}}}}}
	requireOK(t, m.Apply(t.Context(), d))
	h.commands = nil
	h.fail = "exportfs"
	d.Serve[0].Clients = nil
	r := m.Apply(t.Context(), d)
	if r[0].Error == "" {
		t.Fatal("missing update failure")
	}
	stopped := false
	for _, c := range h.commands {
		if c.Program == "systemctl" && reflect.DeepEqual(c.Args, []string{"stop", "iii-mesh-smbd"}) {
			stopped = true
		}
		if c.Program == "exportfs" && !stopped {
			t.Fatal("old SMB access survived failed membership update")
		}
	}
}
func TestServerRemovalRetriesPartialUserDeletion(t *testing.T) {
	h := hostFixture()
	m := managerFixture(t, "linux", t.TempDir(), h)
	d := fixtureDesired()
	d.Mount = nil
	d.Serve = []Export{{Name: "proj", Clients: []Client{{Name: "win", Addr: netip.MustParseAddr("fd31:32:33::9"), SMB: &SMB{User: "iii-client", Password: strings.Repeat("e", 64)}}}}}
	requireOK(t, m.Apply(t.Context(), d))
	h.fail = "userdel"
	if err := m.Drop(t.Context()); err == nil {
		t.Fatal("missing cleanup failure")
	}
	h.fail = ""
	h.commands = nil
	delete(h.files, "/etc/passwd")
	if err := m.Drop(t.Context()); err != nil {
		t.Fatal(err)
	}
	for _, c := range h.commands {
		if c.Program == "userdel" {
			t.Fatal("retried deletion of already-absent user")
		}
	}
}

func TestWindowsStableLettersCredentialLifetimeAndSessionErrors(t *testing.T) {
	h := hostFixture()
	dir := t.TempDir()
	m := managerFixture(t, "windows", dir, h)
	d := fixtureDesired()
	pw := strings.Repeat("f", 64)
	d.Mount[0].SMB = &SMB{User: "iii-client", Password: pw}
	second := d.Mount[0]
	second.Name = "archive"
	d.Mount = append(d.Mount, second)
	h.mounts["W:"] = "local disk"
	requireOK(t, m.Apply(t.Context(), d))
	if h.mounts["V:"] != `\\server.iii.internal\proj` || h.mounts["U:"] != `\\server.iii.internal\archive` {
		t.Fatal(h.mounts)
	}
	b, err := os.ReadFile(filepath.Join(dir, "drives.json"))
	if err != nil || strings.Contains(string(b), pw) {
		t.Fatal("journal retained SMB password", err)
	}
	m.Close()
	again := managerFixture(t, "windows", dir, h)
	requireOK(t, again.Apply(t.Context(), d))
	d.Mount = d.Mount[1:]
	h.commands = nil
	requireOK(t, again.Apply(t.Context(), d))
	for _, c := range h.commands {
		if c.Program == "cmdkey" && strings.HasPrefix(c.Args[0], "/delete:") {
			t.Fatal("removed credentials of retained share")
		}
	}
	h.fail = "net"
	if err := again.Drop(t.Context()); err == nil {
		t.Fatal("missing original-session cleanup failure")
	}
	h.fail = ""
	if err := again.Drop(t.Context()); err != nil {
		t.Fatal(err)
	}
	if len(h.mounts) != 1 || h.mounts["W:"] != "local disk" {
		t.Fatal("changed foreign drive", h.mounts)
	}
}

func TestPartialSambaInstallMasksAreRecovered(t *testing.T) {
	h := hostFixture()
	dir := t.TempDir()
	m := managerFixture(t, "linux", dir, h)
	m.owned.Server = true
	m.owned.Masks = []string{"smbd.service"}
	if err := m.save(); err != nil {
		t.Fatal(err)
	}
	m.Close()
	again := managerFixture(t, "linux", dir, h)
	if err := again.Drop(t.Context()); err != nil {
		t.Fatal(err)
	}
	found := false
	for _, c := range h.commands {
		if c.Program == "systemctl" && reflect.DeepEqual(c.Args, []string{"unmask", "--runtime", "smbd.service"}) {
			found = true
		}
	}
	if !found {
		t.Fatal("crash left an owned runtime mask")
	}
}
func TestNativeCommandCancellationStopsDescendants(t *testing.T) {
	ctx, cancel := context.WithTimeout(t.Context(), 40*time.Millisecond)
	defer cancel()
	start := time.Now()
	h := &nativeHost{}
	_, err := h.Run(ctx, Command{Program: "sh", Args: []string{"-c", "sleep 10 & wait"}})
	if err == nil || time.Since(start) > time.Second {
		t.Fatalf("command descendants survived cancellation: %s %v", time.Since(start), err)
	}
}

func TestWindowsMappingComparisonIgnoresCase(t *testing.T) {
	h := hostFixture()
	m := managerFixture(t, "windows", t.TempDir(), h)
	d := fixtureDesired()
	d.Mount[0].SMB = &SMB{User: "iii-client", Password: strings.Repeat("a", 64)}
	requireOK(t, m.Apply(t.Context(), d))
	h.mounts["W:"] = `\\SERVER.III.INTERNAL\PROJ`
	requireOK(t, m.Apply(t.Context(), d))
	if err := m.Drop(t.Context()); err != nil {
		t.Fatal(err)
	}
}

func TestServerRestoresSuccessfulStateAfterPartialReconfiguration(t *testing.T) {
	for _, program := range []string{"exportfs", "smbpasswd"} {
		t.Run(program, func(t *testing.T) {
			h := hostFixture()
			m := managerFixture(t, "linux", t.TempDir(), h)
			a := fixtureDesired()
			a.Mount = nil
			a.Serve = []Export{{Name: "proj", Clients: []Client{{Name: "a", Addr: netip.MustParseAddr("fd31:32:33::9"), SMB: &SMB{User: "iii-a", Password: strings.Repeat("a", 64)}}}}}
			requireOK(t, m.Apply(t.Context(), a))
			b := fixtureDesired()
			b.Mount = nil
			b.Serve = []Export{{Name: "proj", Clients: []Client{{Name: "b", Addr: netip.MustParseAddr("fd31:32:33::10"), SMB: &SMB{User: "iii-b", Password: strings.Repeat("b", 64)}}}}}
			h.fail = program
			failed := m.Apply(t.Context(), b)
			if len(failed) != 1 || failed[0].State != "error" {
				t.Fatal("partial change falsely succeeded", failed)
			}
			h.fail = ""
			requireOK(t, m.Apply(t.Context(), a))
			exports := string(h.files[exportsFile])
			conf := string(h.files[m.smbConf()])
			passwd := string(h.files["/etc/passwd"])
			if !strings.Contains(exports, "fd31:32:33::9(") || strings.Contains(exports, "fd31:32:33::10(") || !strings.Contains(conf, "valid users = iii-a\n") || strings.Contains(conf, "iii-b") || !strings.Contains(passwd, "iii-a:") || strings.Contains(passwd, "iii-b:") {
				t.Fatalf("rollback did not restore actual resources: exports=%s conf=%s passwd=%s", exports, conf, passwd)
			}
			requireOK(t, m.Apply(t.Context(), a))
		})
	}
}
func TestServerRestoresAfterPartialTeardown(t *testing.T) {
	h := hostFixture()
	m := managerFixture(t, "linux", t.TempDir(), h)
	a := fixtureDesired()
	a.Mount = nil
	a.Serve = []Export{{Name: "proj", Clients: []Client{{Name: "client", Addr: netip.MustParseAddr("fd31:32:33::9")}}}}
	requireOK(t, m.Apply(t.Context(), a))
	h.fail = "exportfs"
	r := m.Apply(t.Context(), Desired{})
	if len(r) != 1 || r[0].Error == "" {
		t.Fatal(r)
	}
	h.fail = ""
	requireOK(t, m.Apply(t.Context(), a))
	if !strings.Contains(string(h.files[exportsFile]), "fd31:32:33::9(") {
		t.Fatal("partial teardown was reported restored without exports")
	}
}

func TestNodeNamesRemainValidAcrossMountAndJournal(t *testing.T) {
	for _, platform := range []string{"linux", "darwin", "windows"} {
		for _, server := range []string{"nas--one", "nas-"} {
			t.Run(platform+"/"+server, func(t *testing.T) {
				h := hostFixture()
				dir := t.TempDir()
				m := managerFixture(t, platform, dir, h)
				d := fixtureDesired()
				d.Mount[0].Server = server
				d.Mount[0].Host = server + ".iii.internal"
				if platform == "windows" {
					d.Mount[0].SMB = &SMB{User: "iii-client", Password: strings.Repeat("a", 64)}
				}
				requireOK(t, m.Apply(t.Context(), d))
				m.Close()
				again := managerFixture(t, platform, dir, h)
				if err := again.Drop(t.Context()); err != nil {
					t.Fatal(err)
				}
			})
		}
	}
	for _, server := range []string{"../nas", "-nas", "7nas", "nas.example", "NAS"} {
		h := hostFixture()
		m := managerFixture(t, "linux", t.TempDir(), h)
		d := fixtureDesired()
		d.Mount[0].Server = server
		d.Mount[0].Host = server + ".iii.internal"
		r := m.Apply(t.Context(), d)
		if len(r) != 1 || r[0].Error == "" || len(h.commands) != 0 {
			t.Fatal("invalid node reached OS boundary", server, r)
		}
	}
}

func testLogon() supervisor.OperatorLogon {
	return supervisor.OperatorLogon{SID: "S-1-5-21-1-2-3-1001", SessionID: 2, AuthenticationID: 0x100000002, LogonTime: 133000000000000000}
}
func (h *testHost) CaptureLogon() (supervisor.OperatorLogon, error) { return testLogon(), nil }
func (h *testHost) BindLogon(supervisor.OperatorLogon) error        { return nil }

type sessionDriveHost struct {
	*testHost
	active           supervisor.OperatorLogon
	sessions         map[uint32]supervisor.OperatorLogon
	namespaces       map[uint32]map[string]string
	credentials      map[uint32]bool
	bound            *supervisor.OperatorLogon
	captures         int
	afterEnumeration func()
	beforeCommand    func(Command)
}

func newSessionDriveHost() *sessionDriveHost {
	a := testLogon()
	return &sessionDriveHost{testHost: hostFixture(), active: a, sessions: map[uint32]supervisor.OperatorLogon{a.SessionID: a}, namespaces: map[uint32]map[string]string{a.SessionID: {}}, credentials: map[uint32]bool{}}
}
func (h *sessionDriveHost) CaptureLogon() (supervisor.OperatorLogon, error) {
	h.captures++
	return h.active, nil
}
func (h *sessionDriveHost) BindLogon(l supervisor.OperatorLogon) error { h.bound = &l; return nil }
func (h *sessionDriveHost) current() (supervisor.OperatorLogon, error) {
	l := h.active
	if h.bound != nil {
		l = *h.bound
	}
	actual, ok := h.sessions[l.SessionID]
	if !ok || actual != l {
		return l, errors.New("original operator logon is unavailable or reused")
	}
	return l, nil
}
func (h *sessionDriveHost) Mounts(context.Context) (map[string]string, error) {
	l, err := h.current()
	if err != nil {
		return nil, err
	}
	at := maps.Clone(h.namespaces[l.SessionID])
	if h.afterEnumeration != nil {
		f := h.afterEnumeration
		h.afterEnumeration = nil
		f()
	}
	return at, nil
}
func (h *sessionDriveHost) Run(ctx context.Context, c Command) (string, error) {
	l, err := h.current()
	if err != nil {
		return "", err
	}
	if !c.Operator {
		return "", errors.New("unbound Windows command")
	}
	if h.beforeCommand != nil {
		h.beforeCommand(c)
	}
	h.testHost.mounts = h.namespaces[l.SessionID]
	if c.Program == "cmdkey" {
		switch {
		case c.Args[0] == "/list":
			if h.credentials[l.SessionID] {
				return "server.iii.internal", nil
			}
		case strings.HasPrefix(c.Args[0], "/add:"):
			h.credentials[l.SessionID] = true
		case strings.HasPrefix(c.Args[0], "/delete:"):
			h.credentials[l.SessionID] = false
		}
	}
	return h.testHost.Run(ctx, c)
}
func windowsDesired() Desired {
	d := fixtureDesired()
	d.Mount[0].SMB = &SMB{User: "iii-client", Password: strings.Repeat("a", 64)}
	return d
}
func readDriveJournal(t *testing.T, dir string) map[string]json.RawMessage {
	t.Helper()
	b, err := os.ReadFile(filepath.Join(dir, "drives.json"))
	if err != nil {
		t.Fatal(err)
	}
	var j map[string]json.RawMessage
	if err = json.Unmarshal(b, &j); err != nil {
		t.Fatal(err)
	}
	return j
}
func TestWindowsCleanupRemainsInOriginalLogon(t *testing.T) {
	h := newSessionDriveHost()
	dir := t.TempDir()
	m := managerFixture(t, "windows", dir, h)
	requireOK(t, m.Apply(t.Context(), windowsDesired()))
	m.Close()
	a := testLogon()
	b := a
	b.SessionID = 3
	b.AuthenticationID++
	b.LogonTime++
	h.active = b
	h.sessions[b.SessionID] = b
	h.namespaces[b.SessionID] = map[string]string{"W:": `\\foreign\data`}
	h.bound = nil
	again := managerFixture(t, "windows", dir, h)
	if err := again.Drop(t.Context()); err != nil {
		t.Fatal(err)
	}
	if len(h.namespaces[a.SessionID]) != 0 || h.credentials[a.SessionID] || h.namespaces[b.SessionID]["W:"] != `\\foreign\data` || h.captures != 1 {
		t.Fatal("cleanup changed logons", h.namespaces, h.credentials, h.captures)
	}
	if _, ok := readDriveJournal(t, dir)["logon"]; ok {
		t.Fatal("fully cleaned ownership retained logon binding")
	}
}
func TestWindowsUnavailableOrReusedLogonRetainsOwnership(t *testing.T) {
	for _, failure := range []string{"disappeared", "session reused", "LUID reused", "between enumeration and mutation", "legacy journal"} {
		t.Run(failure, func(t *testing.T) {
			h := newSessionDriveHost()
			dir := t.TempDir()
			m := managerFixture(t, "windows", dir, h)
			requireOK(t, m.Apply(t.Context(), windowsDesired()))
			m.Close()
			a := testLogon()
			b := a
			b.SessionID = 3
			b.AuthenticationID++
			b.LogonTime++
			h.active = b
			h.sessions[b.SessionID] = b
			h.namespaces[b.SessionID] = map[string]string{}
			h.bound = nil
			switch failure {
			case "disappeared":
				delete(h.sessions, a.SessionID)
			case "session reused":
				reuse := a
				reuse.AuthenticationID++
				h.sessions[a.SessionID] = reuse
			case "LUID reused":
				reuse := a
				reuse.LogonTime++
				h.sessions[a.SessionID] = reuse
			case "between enumeration and mutation":
				h.afterEnumeration = func() { delete(h.sessions, a.SessionID) }
			case "legacy journal":
				j := readDriveJournal(t, dir)
				delete(j, "logon")
				data, _ := json.Marshal(j)
				if err := os.WriteFile(filepath.Join(dir, "drives.json"), data, 0600); err != nil {
					t.Fatal(err)
				}
			}
			before, err := os.ReadFile(filepath.Join(dir, "drives.json"))
			if err != nil {
				t.Fatal(err)
			}
			commands := len(h.commands)
			again := managerFixture(t, "windows", dir, h)
			if err = again.Drop(t.Context()); err == nil {
				t.Fatal("cleanup forgot inaccessible original logon")
			}
			after, _ := os.ReadFile(filepath.Join(dir, "drives.json"))
			if string(after) != string(before) || len(h.namespaces[a.SessionID]) != 1 || len(h.namespaces[b.SessionID]) != 0 || len(h.commands) != commands || h.captures != 1 {
				t.Fatal("failed cleanup lost ownership or crossed logons", string(after), h.namespaces, h.captures)
			}
			// A failed cleanup remains retryable when the original logon is available again.
			if failure != "legacy journal" {
				h.sessions[a.SessionID] = a
				if err = again.Drop(t.Context()); err != nil {
					t.Fatal(err)
				}
				if len(h.namespaces[a.SessionID]) != 0 || h.credentials[a.SessionID] {
					t.Fatal("retry retained owned resources")
				}
			}
		})
	}
}
func TestWindowsLogonPersistedBeforeCredentialMutation(t *testing.T) {
	h := newSessionDriveHost()
	h.fail = "net"
	dir := t.TempDir()
	m := managerFixture(t, "windows", dir, h)
	checked := false
	h.beforeCommand = func(c Command) {
		if c.Program != "cmdkey" || !strings.HasPrefix(c.Args[0], "/add:") {
			return
		}
		var j journal
		data, err := os.ReadFile(filepath.Join(dir, "drives.json"))
		if err != nil {
			t.Fatal(err)
		}
		if err = json.Unmarshal(data, &j); err != nil {
			t.Fatal(err)
		}
		if j.Logon == nil || *j.Logon != testLogon() || len(j.Mounts) != 1 {
			t.Fatal("credential mutation preceded ownership persistence", j)
		}
		checked = true
	}
	r := m.Apply(t.Context(), windowsDesired())
	if len(r) != 1 || r[0].Error == "" {
		t.Fatal(r)
	}
	if !checked {
		t.Fatal("credential mutation was not exercised")
	}
	var saved supervisor.OperatorLogon
	j := readDriveJournal(t, dir)
	if err := json.Unmarshal(j["logon"], &saved); err != nil {
		t.Fatal(err)
	}
	if saved != testLogon() || !h.credentials[saved.SessionID] || len(m.owned.Mounts) != 1 {
		t.Fatal("partial credential ownership lacks original logon", saved, m.owned)
	}
	m.Close()
	h.bound = nil
	h.fail = ""
	again := managerFixture(t, "windows", dir, h)
	if err := again.Drop(t.Context()); err != nil {
		t.Fatal(err)
	}
	if h.credentials[saved.SessionID] {
		t.Fatal("orphaned credential")
	}
}

func TestWindowsMappingEnumerationParsesNativeOutput(t *testing.T) {
	out := "New connections will not be remembered.\r\nStatus       Local     Remote                    Network\r\n-------------------------------------------------------------------------------\r\nOK           W:        \\\\server.iii.internal\\proj Microsoft Windows Network\r\nDisconnected V:        \\\\foreign\\share             Microsoft Windows Network\r\nOK                     \\\\foreign\\IPC$              Microsoft Windows Network\r\nThe command completed successfully.\r\n"
	want := map[string]string{"W:": `\\server.iii.internal\proj`, "V:": `\\foreign\share`}
	if got := parseWindowsMappings(out); !reflect.DeepEqual(got, want) {
		t.Fatal("native mappings were not enumerated", got, want)
	}
}

func TestWindowsLogicalDriveEnumerationPreservesMappings(t *testing.T) {
	at := map[string]string{"W:": `\\server.iii.internal\proj`}
	if err := addWindowsDriveLetters(at, "C:\\\r\nW:\\\r\n"); err != nil {
		t.Fatal(err)
	}
	if at["C:"] != "local disk" || at["W:"] != `\\server.iii.internal\proj` {
		t.Fatal(at)
	}
	if err := addWindowsDriveLetters(at, "Access denied"); err == nil {
		t.Fatal("malformed enumeration permitted letter allocation")
	}
}
func TestWindowsJournalRejectsInvalidLogonEncoding(t *testing.T) {
	for _, bad := range []string{`{"sid":"S-1-5-21-1-2-3-1001","session_id":0,"authentication_id":"7","logon_time":"133000000000000000"}`, `{"sid":"S-1-5-18","session_id":2,"authentication_id":"7","logon_time":"133000000000000000"}`, `{"sid":"S-1-5-21-1-2-3-1001","session_id":2,"authentication_id":"18446744073709551616","logon_time":"133000000000000000"}`, `{"sid":"S-1-5-21-1-2-3-1001","session_id":2,"authentication_id":7,"logon_time":"133000000000000000"}`, `{"sid":"S-1-5-21-1-2-3-1001","session_id":2,"authentication_id":"7","logon_time":"-1"}`} {
		dir := t.TempDir()
		data := []byte(`{"mounts":{},"logon":` + bad + `}`)
		if err := os.WriteFile(filepath.Join(dir, "drives.json"), data, 0600); err != nil {
			t.Fatal(err)
		}
		h := newSessionDriveHost()
		m, err := NewWithHost(Options{StateDir: dir, Platform: "windows", Operator: "alice"}, h)
		if err == nil {
			m.Close()
			t.Fatal("invalid saved logon accepted", bad)
		}
		after, _ := os.ReadFile(filepath.Join(dir, "drives.json"))
		if string(after) != string(data) || len(h.commands) > 0 || h.captures > 0 {
			t.Fatal("invalid saved identity changed resources")
		}
	}
}
