package coordinator

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/sha256"
	"encoding/binary"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"net/netip"
	"os"
	"path/filepath"
	"reflect"
	"regexp"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/iii-hq/workers/mesh/internal/join"
	"github.com/iii-hq/workers/mesh/internal/relay"
	"tailscale.com/ipn"
	"tailscale.com/tailcfg"
	"tailscale.com/tsnet"
	"tailscale.com/types/key"
	"tailscale.com/util/zstdframe"
)

func newCoordinator(t *testing.T, dir string) *Coordinator {
	t.Helper()
	c, err := New(dir, func(secret, addr string) (string, error) {
		if secret != "" {
			return "", nil
		}
		return "", errors.New("missing secret")
	})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = c.Close() })
	return c
}
func registerNode(t *testing.T, c *Coordinator, name string) (key.MachinePublic, key.NodePublic) {
	t.Helper()
	m, k := key.NewMachine().Public(), key.NewNode().Public()
	secret := rand.Text()
	if name == "hub" {
		secret = c.self
	}
	r := c.register(m, "127.0.0.1", tailcfg.RegisterRequest{NodeKey: k, Hostinfo: &tailcfg.Hostinfo{Hostname: name}, Auth: &tailcfg.RegisterResponseAuth{AuthKey: secret}})
	if !r.MachineAuthorized || r.Error != "" {
		t.Fatalf("register %s: %+v", name, r)
	}
	return m, k
}
func TestLinksDecideVisibility(t *testing.T) {
	c := newCoordinator(t, t.TempDir())
	for _, name := range []string{"hub", "a", "b", "c"} {
		registerNode(t, c, name)
	}
	c.Link([][2]string{{"a", "b"}})
	for name, want := range map[string][]string{"a": {"b", "hub"}, "c": {"hub"}} {
		sent, host := map[tailcfg.NodeID]uint64{}, ""
		r, ok := c.delta(c.byName(name), sent, &host, true, nil)
		if !ok {
			t.Fatal("missing map")
		}
		var got []string
		for _, p := range r.Peers {
			got = append(got, p.Name)
		}
		slices.Sort(got)
		if !reflect.DeepEqual(got, want) {
			t.Fatalf("%s peers %v, want %v", name, got, want)
		}
		if name == "a" {
			c.Link(nil)
			r, _ = c.delta(c.byName(name), sent, &host, false, nil)
			if !slices.Contains(r.PeersRemoved, c.byName("b").ID) {
				t.Fatal("unlinked peer not removed")
			}
		}
	}
}
func TestSanitizeName(t *testing.T) {
	for in, want := range map[string]string{"DESKTOP_7Q2": "desktop-7q2", "123-pc": "n-123-pc", "café-box": "caf-box", "": "node", strings.Repeat("a", 40): strings.Repeat("a", 31)} {
		if got := SanitizeName(in); got != want {
			t.Errorf("%q = %q, want %q", in, got, want)
		}
	}
}
func TestReservedAndTakenNamesGetSuffix(t *testing.T) {
	for _, name := range []string{"mesh", "hub", "engine", "iii", "configuration", "harness", "shell", "coder", "laptop"} {
		if got := AllocateName(name, func(s string) bool { return s == "laptop" }); got != name+"-2" {
			t.Errorf("%s = %s", name, got)
		}
	}
	got := AllocateName(strings.Repeat("a", 40), func(s string) bool { return len(s) == 31 && !strings.HasSuffix(s, "-2") })
	if !regexp.MustCompile(`^[a-z][a-z0-9-]{0,30}$`).MatchString(got) || !strings.HasSuffix(got, "-2") {
		t.Fatal(got)
	}
	c := newCoordinator(t, t.TempDir())
	c.SetNameTaken(func(s string) bool { return s == "laptop" })
	registerNode(t, c, "laptop")
	if c.Nodes()[0].Name != "laptop-2" {
		t.Fatal(c.Nodes())
	}
}
func TestRemoveRefusesKeysAndCyclesCode(t *testing.T) {
	dir := t.TempDir()
	c := newCoordinator(t, dir)
	m, k := registerNode(t, c, "a")
	before := c.Code()
	if err := c.Remove("a"); err != nil {
		t.Fatal(err)
	}
	if c.Known(k) || c.Code() == before {
		t.Fatal("removal did not revoke key and rotate code")
	}
	if err := c.Close(); err != nil {
		t.Fatal(err)
	}
	c = newCoordinator(t, dir)
	for _, pair := range []struct {
		m key.MachinePublic
		k key.NodePublic
	}{{m, k}, {key.NewMachine().Public(), k}, {m, key.NewNode().Public()}} {
		r := c.register(pair.m, "127.0.0.1", tailcfg.RegisterRequest{NodeKey: pair.k, Auth: &tailcfg.RegisterResponseAuth{AuthKey: rand.Text()}})
		if r.MachineAuthorized {
			t.Fatal("removed keys authorized again")
		}
	}
	// A logout must not erase refused keys.
	c.register(m, "127.0.0.1", tailcfg.RegisterRequest{NodeKey: k, Expiry: time.Now().Add(-time.Minute)})
	r := c.register(m, "127.0.0.1", tailcfg.RegisterRequest{NodeKey: k, Auth: &tailcfg.RegisterResponseAuth{AuthKey: rand.Text()}})
	if r.MachineAuthorized {
		t.Fatal("logout erased refusal")
	}
}
func TestHubRestartKeepsNodes(t *testing.T) {
	dir := t.TempDir()
	c := newCoordinator(t, dir)
	_, k := registerNode(t, c, "a")
	id, control, relay, prefix, code := c.Identity(), c.Public(), c.RelayKey(), c.Prefix(), c.Code()
	value := []byte(rand.Text())
	if err := c.StateStore().WriteState(ipn.StateKey("test-state"), value); err != nil {
		t.Fatal(err)
	}
	if err := c.Close(); err != nil {
		t.Fatal(err)
	}
	c = newCoordinator(t, dir)
	if len(c.Nodes()) != 1 || c.Nodes()[0].Name != "a" || !c.Known(k) {
		t.Fatal(c.Nodes())
	}
	if !bytes.Equal(c.Identity(), id) || c.Public() != control || !c.RelayKey().Equal(relay) || c.Prefix() != prefix || c.Code() != code {
		t.Fatal("persistent identity changed")
	}
	value2, err := c.StateStore().ReadState(ipn.StateKey("test-state"))
	if err != nil || !bytes.Equal(value, value2) {
		t.Fatalf("store: %q %v", value2, err)
	}
	fi, err := os.Stat(filepath.Join(dir, "state.json"))
	if err != nil || fi.Mode().Perm() != 0600 {
		t.Fatalf("state permissions: %v %v", fi, err)
	}
	if prefix.Bits() != 48 || prefix.Addr().As16()[0] != 0xfd {
		t.Fatal(prefix)
	}
	entries, _ := os.ReadDir(dir)
	if len(entries) != 1 || entries[0].Name() != "state.json" {
		t.Fatal(entries)
	}
}
func TestFailedWritesDoNotAuthorizeOrRotate(t *testing.T) {
	c := newCoordinator(t, t.TempDir())
	_, k := registerNode(t, c, "a")
	old := c.Code()
	file := c.file
	c.file = filepath.Join(file, "impossible")
	if err := c.Remove("a"); err == nil {
		t.Fatal("write unexpectedly succeeded")
	}
	if !c.Known(k) || c.Code() != old {
		t.Fatal("failed transaction modified live state")
	}
	m, k2 := key.NewMachine().Public(), key.NewNode().Public()
	r := c.register(m, "127.0.0.1", tailcfg.RegisterRequest{NodeKey: k2, Hostinfo: &tailcfg.Hostinfo{Hostname: "b"}, Auth: &tailcfg.RegisterResponseAuth{AuthKey: rand.Text()}})
	if r.Error == "" || c.Known(k2) {
		t.Fatal("failed registration is authorized")
	}
	if err := c.StateStore().WriteState("failed", []byte("value")); err == nil {
		t.Fatal("write unexpectedly succeeded")
	}
	if _, err := c.StateStore().ReadState("failed"); !errors.Is(err, ipn.ErrStateNotExist) {
		t.Fatalf("failed store write visible: %v", err)
	}
	c.file = file
}
func TestIdentityAndMapWireFormat(t *testing.T) {
	c := newCoordinator(t, t.TempDir())
	m, k := registerNode(t, c, "a")
	registerNode(t, c, "hub")
	mux := http.NewServeMux()
	c.Handle(mux)
	server := httptest.NewServer(mux)
	defer server.Close()
	doc, err := join.FetchIdentity(t.Context(), server.URL)
	if err != nil {
		t.Fatal(err)
	}
	if err := doc.Verify(c.Identity().Public().(ed25519.PublicKey)); err != nil || doc.Control != c.Public() {
		t.Fatalf("identity: %v", err)
	}
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	b, _ := json.Marshal(tailcfg.MapRequest{NodeKey: k, Stream: true, Compress: "zstd", DiscoKey: key.NewDisco().Public(), Endpoints: []netip.AddrPort{netip.MustParseAddrPort("127.0.0.1:41643")}})
	req := httptest.NewRequest("POST", "/machine/map", bytes.NewReader(b)).WithContext(ctx)
	w := &firstFrame{ResponseRecorder: httptest.NewRecorder(), cancel: cancel}
	c.serveMap(w, req, m)
	body := w.Body.Bytes()
	if len(body) < 4 || int(binary.LittleEndian.Uint32(body)) != len(body)-4 {
		t.Fatal("invalid framing")
	}
	raw, err := zstdframe.AppendDecode(nil, body[4:])
	if err != nil {
		t.Fatal(err)
	}
	var response tailcfg.MapResponse
	if err := json.Unmarshal(raw, &response); err != nil {
		t.Fatal(err)
	}
	if response.Domain != "iii.internal" || len(response.Peers) != 1 || response.Peers[0].Name != "hub" {
		t.Fatalf("map: %+v", response)
	}
}

type firstFrame struct {
	*httptest.ResponseRecorder
	cancel context.CancelFunc
}

func (w *firstFrame) Flush() { w.ResponseRecorder.Flush(); w.cancel() }
func TestRelayMapContainsOnlyHub(t *testing.T) {
	c := newCoordinator(t, t.TempDir())
	c.Relay("https://relay.example:8443")
	r := c.relayMap()
	if len(r.Regions) != 1 || len(r.Regions[900].Nodes) != 1 {
		t.Fatal(r)
	}
	n := r.Regions[900].Nodes[0]
	if n.HostName != "relay.example" || n.DERPPort != 8443 || n.STUNPort != -1 {
		t.Fatal(n)
	}
}
func TestListenUserspaceSurvivesRestart(t *testing.T) {
	dir := t.TempDir()
	var hubKey key.NodePublic
	var hubAddr netip.Addr
	for iteration := 0; iteration < 2; iteration++ {
		c := newCoordinator(t, dir)
		admission := httptest.NewServer(relay.AdmitHandler(c.Known))
		defer admission.Close()
		derp := relay.NewWithKey(c.RelayKey(), admission.URL)
		defer derp.Close()
		relayMux := http.NewServeMux()
		relay.Handle(relayMux, derp)
		relayServer := httptest.NewTLSServer(relayMux)
		defer relayServer.Close()
		pin := sha256.Sum256(relayServer.Certificate().Raw)
		mux := http.NewServeMux()
		c.Handle(mux)
		server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			r.Header.Set(DERPURLHeader, relayServer.URL+"/derp")
			r.Header.Set(DERPCertHeader, fmt.Sprintf("sha256-raw:%x", pin))
			mux.ServeHTTP(w, r)
		}))
		defer server.Close()
		ctx, cancel := context.WithTimeout(t.Context(), 25*time.Second)
		ls, err := c.Listen(ctx, server.URL, 49135)
		if err != nil {
			cancel()
			server.Close()
			t.Fatal(err)
		}
		n := c.byName("hub")
		if n == nil {
			t.Fatal("hub not registered")
		}
		if iteration == 0 {
			hubKey = n.Key
			hubAddr = c.address(n.ID)
		} else if n.Key != hubKey || c.address(n.ID) != hubAddr {
			t.Fatal("hub node identity changed")
		}
		peer := &tsnet.Server{Dir: t.TempDir(), Hostname: "peer", ControlURL: server.URL, AuthKey: rand.Text(), Logf: t.Logf, UserLogf: t.Logf}
		defer peer.Close()
		if _, err := peer.Up(ctx); err != nil {
			t.Fatal(err)
		}
		go func() {
			conn, err := ls[0].Accept()
			if err != nil {
				return
			}
			defer conn.Close()
			_, _ = io.Copy(conn, conn)
		}()
		conn, err := peer.Dial(ctx, "tcp", ls[0].Addr().String())
		if err != nil {
			t.Fatal(err)
		}
		_ = conn.SetDeadline(time.Now().Add(5 * time.Second))
		_, err = conn.Write([]byte("mesh"))
		if err != nil {
			t.Fatal(err)
		}
		b := make([]byte, 4)
		if _, err = io.ReadFull(conn, b); err != nil || string(b) != "mesh" {
			t.Fatalf("echo: %q %v", b, err)
		}
		_ = conn.Close()
		_ = peer.Close()
		cancel()
		_ = c.Close()
		server.Close()
		entries, _ := os.ReadDir(dir)
		for _, e := range entries {
			if e.Name() != "state.json" {
				t.Fatalf("unexpected backend state/log file: %s", e.Name())
			}
		}
	}
}

func TestSessionRelayIsPrivateAndPinned(t *testing.T) {
	c := newCoordinator(t, t.TempDir())
	registerNode(t, c, "a")
	registerNode(t, c, "b")
	pin := make([]byte, 32)
	_, _ = rand.Read(pin)
	cert := "sha256-raw:" + fmt.Sprintf("%x", pin)
	h := http.Header{}
	h.Set(DERPURLHeader, "https://127.0.0.1:52345/derp")
	h.Set(DERPCertHeader, cert)
	relay, err := sessionRelay(h)
	if err != nil {
		t.Fatal(err)
	}
	for name, custom := range map[string]*tailcfg.DERPNode{"a": relay, "b": nil} {
		host := ""
		r, _ := c.delta(c.byName(name), map[tailcfg.NodeID]uint64{}, &host, true, custom)
		nodes := r.DERPMap.Regions[900].Nodes
		if name == "a" {
			if len(nodes) != 1 || nodes[0].HostName != "127.0.0.1" || nodes[0].DERPPort != 52345 || nodes[0].CertName != cert || nodes[0].InsecureForTests || nodes[0].STUNPort != -1 {
				t.Fatalf("relay: %+v", nodes)
			}
		} else if len(nodes) != 0 {
			t.Fatal("another session inherited relay")
		}
	}
	for _, url := range []string{"http://127.0.0.1:52345/derp", "https://localhost:52345/derp", "https://192.0.2.1:52345/derp", "https://127.0.0.1/derp", "https://127.0.0.1:0/derp", "https://127.0.0.1:65536/derp", "https://user@127.0.0.1:52345/derp", "https://127.0.0.1:52345/other", "https://127.0.0.1:52345/derp?x=1", "https://127.0.0.1:52345/derp#x", "https://127.0.0.1:52345/derp#"} {
		h.Set(DERPURLHeader, url)
		if _, err := sessionRelay(h); err == nil {
			t.Errorf("accepted %s", url)
		}
	}
	h.Set(DERPURLHeader, "https://[::1]:52345/derp")
	if _, err := sessionRelay(h); err != nil {
		t.Fatal(err)
	}
	for _, pin := range []string{"", "sha256-raw:xx", strings.Repeat("a", 64)} {
		h.Set(DERPCertHeader, pin)
		if _, err := sessionRelay(h); err == nil {
			t.Errorf("accepted pin %q", pin)
		}
	}
}
func TestFailedMapUpdateIsNotPublished(t *testing.T) {
	c := newCoordinator(t, t.TempDir())
	_, k := registerNode(t, c, "a")
	n := c.byKey(k)
	file := c.file
	c.file = filepath.Join(file, "impossible")
	old := n.Disco
	c.update(n, tailcfg.MapRequest{DiscoKey: key.NewDisco().Public(), Endpoints: []netip.AddrPort{netip.MustParseAddrPort("127.0.0.1:41643")}})
	if n.Disco != old || len(n.Endpoints) != 0 {
		t.Fatal("unpersisted map published")
	}
	c.file = file
}
func TestRegisterCallbackCanReadCoordinator(t *testing.T) {
	var c *Coordinator
	var err error
	secret := rand.Text()
	c, err = New(t.TempDir(), func(s, addr string) (string, error) {
		if c.Code() == "" || len(c.Identity()) == 0 || s != secret {
			return "", errors.New("bad secret")
		}
		return "", nil
	})
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	done := make(chan tailcfg.RegisterResponse, 1)
	go func() {
		done <- c.register(key.NewMachine().Public(), "127.0.0.1", tailcfg.RegisterRequest{NodeKey: key.NewNode().Public(), Auth: &tailcfg.RegisterResponseAuth{AuthKey: secret}})
	}()
	select {
	case r := <-done:
		if !r.MachineAuthorized {
			t.Fatal(r)
		}
	case <-time.After(time.Second):
		t.Fatal("reentrant join callback deadlocked")
	}
}
func TestRegistrationValidatedBeforeRotationIsRejected(t *testing.T) {
	c := newCoordinator(t, t.TempDir())
	registerNode(t, c, "a")
	checked, release := make(chan struct{}), make(chan struct{})
	c.join = func(secret, addr string) (string, error) { close(checked); <-release; return "", nil }
	result := make(chan tailcfg.RegisterResponse, 1)
	go func() {
		result <- c.register(key.NewMachine().Public(), "127.0.0.1", tailcfg.RegisterRequest{NodeKey: key.NewNode().Public(), Auth: &tailcfg.RegisterResponseAuth{AuthKey: rand.Text()}})
	}()
	<-checked
	if err := c.Remove("a"); err != nil {
		t.Fatal(err)
	}
	close(release)
	if r := <-result; r.MachineAuthorized {
		t.Fatal("credentials validated before code rotation were accepted")
	}
}

func TestCorruptStateIsNotReplaced(t *testing.T) {
	dir := t.TempDir()
	file := filepath.Join(dir, "state.json")
	for _, content := range []string{"{", `{"nodes":[]}`} {
		if err := os.WriteFile(file, []byte(content), 0600); err != nil {
			t.Fatal(err)
		}
		if c, err := New(dir, nil); err == nil {
			c.Close()
			t.Fatal("accepted corrupt state")
		}
		after, err := os.ReadFile(file)
		if err != nil || string(after) != content {
			t.Fatal("corrupt state silently replaced")
		}
	}
}
func TestRemoveRefusesPreviousNodeKeys(t *testing.T) {
	c := newCoordinator(t, t.TempDir())
	machine, old := registerNode(t, c, "a")
	next := key.NewNode().Public()
	r := c.register(machine, "127.0.0.1", tailcfg.RegisterRequest{NodeKey: next, OldNodeKey: old, Auth: &tailcfg.RegisterResponseAuth{AuthKey: rand.Text()}})
	if !r.MachineAuthorized {
		t.Fatal(r)
	}
	if err := c.Remove("a"); err != nil {
		t.Fatal(err)
	}
	r = c.register(key.NewMachine().Public(), "127.0.0.1", tailcfg.RegisterRequest{NodeKey: old, Auth: &tailcfg.RegisterResponseAuth{AuthKey: rand.Text()}})
	if r.MachineAuthorized {
		t.Fatal("old node key bypassed removal")
	}
}
func TestStateStoreCopiesAndDeletes(t *testing.T) {
	dir := t.TempDir()
	c := newCoordinator(t, dir)
	s := c.StateStore()
	value := []byte(rand.Text())
	expected := bytes.Clone(value)
	if err := s.WriteState("key", value); err != nil {
		t.Fatal(err)
	}
	value[0] ^= 1
	got, err := s.ReadState("key")
	if err != nil || !bytes.Equal(got, expected) {
		t.Fatal("write retained caller buffer")
	}
	got[0] ^= 1
	got, err = s.ReadState("key")
	if err != nil || !bytes.Equal(got, expected) {
		t.Fatal("read exposed stored buffer")
	}
	if err := s.WriteState("key", nil); err != nil {
		t.Fatal(err)
	}
	c.Close()
	c = newCoordinator(t, dir)
	if _, err := c.StateStore().ReadState("key"); !errors.Is(err, ipn.ErrStateNotExist) {
		t.Fatalf("deleted state reappeared: %v", err)
	}
}
