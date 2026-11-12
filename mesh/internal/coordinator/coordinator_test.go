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
	"sync"
	"sync/atomic"
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
func TestNodesSeeOnlyHub(t *testing.T) {
	c := newCoordinator(t, t.TempDir())
	for _, name := range []string{"hub", "a", "b", "c"} {
		registerNode(t, c, name)
	}
	for name, want := range map[string][]string{"a": {"hub"}, "b": {"hub"}, "c": {"hub"}, "hub": {"a", "b", "c"}} {
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
	if err := c.update(n, tailcfg.MapRequest{DiscoKey: key.NewDisco().Public(), Endpoints: []netip.AddrPort{netip.MustParseAddrPort("127.0.0.1:41643")}}); err == nil {
		t.Fatal("failed write did not return an error")
	}
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

// newCodeCoordinator checks the join code the way the hub's callback does.
func newCodeCoordinator(t *testing.T, dir string) *Coordinator {
	t.Helper()
	var c *Coordinator
	c, err := New(dir, func(secret, addr string) (string, error) {
		if !join.IsTokenSecret(secret) && secret != join.Secret(c.Identity(), c.Code()) {
			return "", join.ErrCode
		}
		return "", nil
	})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = c.Close() })
	return c
}
func codeRequest(nodeKey key.NodePublic, secret string) tailcfg.RegisterRequest {
	return tailcfg.RegisterRequest{NodeKey: nodeKey, Auth: &tailcfg.RegisterResponseAuth{AuthKey: secret}}
}
func TestJoinCodeWorksOnce(t *testing.T) {
	dir := t.TempDir()
	c := newCodeCoordinator(t, dir)
	old := c.Code()
	secret := join.Secret(c.Identity(), old)
	file := c.file
	c.file = filepath.Join(file, "impossible")
	if r := c.register(key.NewMachine().Public(), "127.0.0.1", codeRequest(key.NewNode().Public(), secret)); r.MachineAuthorized || c.Code() != old {
		t.Fatalf("failed save authorized or spent the code: %+v", r)
	}
	c.file = file
	var accepted atomic.Int32
	var wg sync.WaitGroup
	machines := make([]key.MachinePublic, 12)
	nodeKeys := make([]key.NodePublic, 12)
	for i := range machines {
		machines[i], nodeKeys[i] = key.NewMachine().Public(), key.NewNode().Public()
		wg.Go(func() {
			if c.register(machines[i], "127.0.0.1", codeRequest(nodeKeys[i], secret)).MachineAuthorized {
				accepted.Add(1)
			}
		})
	}
	wg.Wait()
	if accepted.Load() != 1 || len(c.Nodes()) != 1 || c.Code() == old {
		t.Fatalf("accepted %d nodes %d, code replaced %v", accepted.Load(), len(c.Nodes()), c.Code() != old)
	}
	if c.register(key.NewMachine().Public(), "127.0.0.1", codeRequest(key.NewNode().Public(), secret)).MachineAuthorized {
		t.Fatal("spent code joined another machine")
	}
	if err := c.Close(); err != nil {
		t.Fatal(err)
	}
	c = newCodeCoordinator(t, dir)
	code := c.Code()
	if code == old {
		t.Fatal("restart restored the spent code")
	}
	n := c.st.Nodes[0]
	// The joined machine reconnects with its keys and its stale secret.
	if !c.register(n.Machine, "127.0.0.1", tailcfg.RegisterRequest{NodeKey: n.Key}).MachineAuthorized ||
		!c.register(n.Machine, "127.0.0.1", codeRequest(n.Key, secret)).MachineAuthorized ||
		!c.register(n.Machine, "127.0.0.1", codeRequest(key.NewNode().Public(), secret)).MachineAuthorized ||
		c.Code() != code || len(c.Nodes()) != 1 {
		t.Fatal("joined machine needed the code or spent it")
	}
	if !c.register(key.NewMachine().Public(), "127.0.0.1", codeRequest(key.NewNode().Public(), join.Secret(c.Identity(), code))).MachineAuthorized || c.Code() == code {
		t.Fatal("replacement code did not join once")
	}
}
func TestTokenAndHubJoinsKeepCode(t *testing.T) {
	c := newCodeCoordinator(t, t.TempDir())
	code := c.Code()
	registerNode(t, c, "hub")
	_, _, secret := issueToken(t, c, join.TokenOptions{})
	if !c.register(key.NewMachine().Public(), "127.0.0.1", tokenRequest(secret, "runner")).MachineAuthorized {
		t.Fatal("token join refused")
	}
	if c.Code() != code {
		t.Fatal("hub or token join replaced the code")
	}
	if !c.register(key.NewMachine().Public(), "127.0.0.1", codeRequest(key.NewNode().Public(), join.Secret(c.Identity(), code))).MachineAuthorized {
		t.Fatal("unspent code refused")
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

func TestHistoricalNodeKeyCannotChangeOwner(t *testing.T) {
	dir := t.TempDir()
	c := newCoordinator(t, dir)
	machine, old := registerNode(t, c, "a")
	next := key.NewNode().Public()
	r := c.register(machine, "127.0.0.1", tailcfg.RegisterRequest{NodeKey: next, Auth: &tailcfg.RegisterResponseAuth{AuthKey: rand.Text()}})
	if !r.MachineAuthorized {
		t.Fatal(r)
	}
	foreign := key.NewMachine().Public()
	r = c.register(foreign, "127.0.0.1", tailcfg.RegisterRequest{NodeKey: old, Hostinfo: &tailcfg.Hostinfo{Hostname: "b"}, Auth: &tailcfg.RegisterResponseAuth{AuthKey: rand.Text()}})
	if r.MachineAuthorized {
		t.Error("historical key reassigned to another machine")
	}
	if err := c.Remove("a"); err != nil {
		t.Fatal(err)
	}
	if c.Known(old) {
		t.Error("reassigned historical key survived removal")
	}
	c.Close()
	c = newCoordinator(t, dir)
	if c.Known(old) {
		t.Fatal("reassigned historical key survived restart")
	}
}

func TestRefusedKeysOverrideExistingRegistrations(t *testing.T) {
	dir := t.TempDir()
	c := newCoordinator(t, dir)
	registerNode(t, c, "hub")
	_, aKey := registerNode(t, c, "a")
	bMachine, bKey := registerNode(t, c, "b")
	// Earlier versions allowed b's current key to overlap a's historical keys.
	c.byKey(aKey).Keys = append(c.byKey(aKey).Keys, bKey)
	if err := c.save(); err != nil {
		t.Fatal(err)
	}
	c.Close()
	c = newCoordinator(t, dir)
	b := c.byKey(bKey)
	addr := c.address(b.ID)
	sent, host := map[tailcfg.NodeID]uint64{}, ""
	if _, ok := c.delta(b, sent, &host, true, nil); !ok {
		t.Fatal("initial map missing")
	}
	if err := c.Remove("a"); err != nil {
		t.Fatal(err)
	}
	if c.Known(bKey) {
		t.Error("refusal did not override existing relay admission")
	}
	if _, ok := c.Node(addr); ok {
		t.Error("refused node still resolves by address")
	}
	if _, ok := c.delta(b, sent, &host, false, nil); ok {
		t.Error("refused existing map stream continues")
	}
	for _, info := range c.Nodes() {
		if info.Name == "b" {
			t.Error("refused node remains listed as active")
		}
	}
	for iteration := 0; iteration < 2; iteration++ {
		b = c.byKey(bKey)
		host = ""
		response, ok := c.delta(b, map[tailcfg.NodeID]uint64{}, &host, true, nil)
		if ok && (response.Node.MachineAuthorized || len(response.Peers) > 0) {
			t.Error("refused node received an authorized map")
		}
		if c.sees(c.byName("hub"), b) {
			t.Error("refused node remains visible to hub")
		}
		r := c.register(bMachine, "127.0.0.1", tailcfg.RegisterRequest{NodeKey: bKey})
		if r.MachineAuthorized || c.Known(bKey) {
			t.Error("existing refused key authorized")
		}
		c.Close()
		c = newCoordinator(t, dir)
	}
}

func TestFailedMapPersistenceRejectsHTTPAndAllowsRetry(t *testing.T) {
	for _, stream := range []bool{false, true} {
		t.Run(fmt.Sprintf("stream=%v", stream), func(t *testing.T) {
			dir := t.TempDir()
			c := newCoordinator(t, dir)
			machine, k := registerNode(t, c, "a")
			n := c.byKey(k)
			oldDisco, oldSeen := n.Disco, n.LastSeen
			req := tailcfg.MapRequest{NodeKey: k, Stream: stream, DiscoKey: key.NewDisco().Public(), Endpoints: []netip.AddrPort{netip.MustParseAddrPort("127.0.0.1:41643")}}
			body, _ := json.Marshal(req)
			request := func() *httptest.ResponseRecorder {
				ctx, cancel := context.WithCancel(t.Context())
				defer cancel()
				response := httptest.NewRecorder()
				c.serveMap(&firstFrame{ResponseRecorder: response, cancel: cancel}, httptest.NewRequest("POST", "/machine/map", bytes.NewReader(body)).WithContext(ctx), machine)
				return response
			}
			file := c.file
			c.file = filepath.Join(file, "impossible")
			failed := request()
			if failed.Code != http.StatusServiceUnavailable {
				t.Errorf("failed disk write acknowledged with HTTP %d", failed.Code)
			}
			if failed.Flushed {
				t.Error("failed disk write started map stream")
			}
			if n.Disco != oldDisco || len(n.Endpoints) != 0 || !n.LastSeen.Equal(oldSeen) {
				t.Fatal("failed write changed peer discovery state")
			}
			c.file = file
			retry := request()
			if retry.Code != http.StatusOK || stream && !retry.Flushed {
				t.Fatalf("retry did not succeed: HTTP %d", retry.Code)
			}
			c.Close()
			c = newCoordinator(t, dir)
			n = c.byKey(k)
			if n.Disco != req.DiscoKey || !slices.Equal(n.Endpoints, req.Endpoints) {
				t.Fatal("acknowledged retry was not persisted")
			}
		})
	}
}

func TestNodeMetadataSurvivesRegistrationActivityAndRestart(t *testing.T) {
	dir := t.TempDir()
	c := newCoordinator(t, dir)
	before := time.Now().UTC()
	machine, k := registerNode(t, c, "a")
	after := time.Now().UTC()
	n := c.byKey(k)
	joined := n.JoinedAt
	if joined.Before(before) || joined.After(after) || !n.LastSeen.Equal(joined) {
		t.Fatalf("registration timestamps: joined=%v seen=%v", joined, n.LastSeen)
	}
	reconnect := time.Now().UTC()
	r := c.register(machine, "127.0.0.1", tailcfg.RegisterRequest{NodeKey: k})
	if !r.MachineAuthorized || n.LastSeen.Before(reconnect) || !n.JoinedAt.Equal(joined) {
		t.Fatal("reconnect did not preserve join time and record activity")
	}
	next := key.NewNode().Public()
	r = c.register(machine, "127.0.0.1", tailcfg.RegisterRequest{NodeKey: next, Auth: &tailcfg.RegisterResponseAuth{AuthKey: rand.Text()}})
	if !r.MachineAuthorized || !n.JoinedAt.Equal(joined) {
		t.Fatal("key rotation replaced durable metadata")
	}
	activity := time.Now().UTC()
	body, _ := json.Marshal(tailcfg.MapRequest{NodeKey: next})
	response := httptest.NewRecorder()
	c.serveMap(response, httptest.NewRequest("POST", "/machine/map", bytes.NewReader(body)), machine)
	if response.Code != http.StatusOK || n.LastSeen.Before(activity) {
		t.Fatal("authenticated map activity was not recorded")
	}
	seen := n.LastSeen
	c.Close()
	c = newCoordinator(t, dir)
	info := c.Nodes()[0]
	if !info.JoinedAt.Equal(joined) || !info.LastSeen.Equal(seen) {
		t.Fatalf("metadata changed on restart: %+v", info)
	}
	raw, err := os.ReadFile(filepath.Join(dir, "state.json"))
	if err != nil {
		t.Fatal(err)
	}
	var persisted struct {
		Nodes []map[string]json.RawMessage `json:"nodes"`
	}
	if err := json.Unmarshal(raw, &persisted); err != nil {
		t.Fatal(err)
	}
	for _, field := range []string{"joined_at", "last_seen"} {
		if _, ok := persisted.Nodes[0][field]; !ok {
			t.Errorf("state is missing %s", field)
		}
	}
}

func TestStateWithRetiredNodeFieldsLoads(t *testing.T) {
	dir := t.TempDir()
	c := newCoordinator(t, dir)
	registerNode(t, c, "a")
	c.Close()
	file := filepath.Join(dir, "state.json")
	raw, err := os.ReadFile(file)
	if err != nil {
		t.Fatal(err)
	}
	var st map[string]json.RawMessage
	var nodes []map[string]json.RawMessage
	if json.Unmarshal(raw, &st) != nil || json.Unmarshal(st["nodes"], &nodes) != nil || len(nodes) != 1 {
		t.Fatalf("state %s", raw)
	}
	nodes[0]["operator"], nodes[0]["target"] = json.RawMessage(`"alice"`), json.RawMessage(`"x86_64-unknown-linux-gnu"`)
	st["nodes"], _ = json.Marshal(nodes)
	raw, _ = json.Marshal(st)
	if err = os.WriteFile(file, raw, 0600); err != nil {
		t.Fatal(err)
	}
	c = newCoordinator(t, dir)
	if got := c.Nodes(); len(got) != 1 || got[0].Name != "a" {
		t.Fatalf("nodes %+v", got)
	}
	if err = c.SetRuntimeMetadata("a", RuntimeMetadata{OS: "linux", Arch: "amd64", Version: "0.1.0"}); err != nil {
		t.Fatal(err)
	}
	raw, _ = os.ReadFile(file)
	nodes = nil
	if json.Unmarshal(raw, &st) != nil || json.Unmarshal(st["nodes"], &nodes) != nil || len(nodes) != 1 {
		t.Fatalf("state %s", raw)
	}
	for _, key := range []string{"operator", "target"} {
		if _, ok := nodes[0][key]; ok {
			t.Fatalf("%s survived the next save", key)
		}
	}
}

func TestLastSeenNeverRegresses(t *testing.T) {
	c := newCoordinator(t, t.TempDir())
	machine, k := registerNode(t, c, "a")
	n := c.byKey(k)
	future := time.Now().UTC().Add(time.Hour)
	n.LastSeen = future
	var wg sync.WaitGroup
	for range 8 {
		wg.Go(func() {
			body, _ := json.Marshal(tailcfg.MapRequest{NodeKey: k})
			response := httptest.NewRecorder()
			c.serveMap(response, httptest.NewRequest("POST", "/machine/map", bytes.NewReader(body)), machine)
			if response.Code != http.StatusOK {
				t.Errorf("activity: HTTP %d", response.Code)
			}
		})
	}
	wg.Wait()
	r := c.register(machine, "127.0.0.1", tailcfg.RegisterRequest{NodeKey: k})
	if !r.MachineAuthorized || !n.LastSeen.Equal(future) {
		t.Fatal("authenticated activity regressed last_seen")
	}
}

func TestAcceptedMachineRotationConcurrentLogout(t *testing.T) {
	c := newCoordinator(t, t.TempDir())
	for range 32 {
		machine, old := registerNode(t, c, "runner")
		c.join = func(string, string) (string, error) { return "", errors.New("new enrollment refused") }
		before := c.st.Next
		start := make(chan struct{})
		results := make(chan tailcfg.RegisterResponse, 2)
		go func() {
			<-start
			results <- c.register(machine, "a", tailcfg.RegisterRequest{NodeKey: key.NewNode().Public(), OldNodeKey: old})
		}()
		go func() {
			<-start
			results <- c.register(machine, "a", tailcfg.RegisterRequest{NodeKey: old, Expiry: time.Now().Add(-time.Minute)})
		}()
		close(start)
		<-results
		<-results
		if c.st.Next != before {
			t.Fatal("rotation recreated a logged-out node without enrollment authority")
		}
		if n := c.byMachine(machine); n != nil {
			c.register(machine, "a", tailcfg.RegisterRequest{NodeKey: n.Key, Expiry: time.Now().Add(-time.Minute)})
		}
		c.join = func(secret, addr string) (string, error) { return "", nil }
	}
}

// A rename picks a name as a join does and is reported as the old name leaving.
func TestRenameAllocatesLikeAJoin(t *testing.T) {
	c := newCoordinator(t, t.TempDir())
	registerNode(t, c, "hub")
	machine, k := registerNode(t, c, "laptop")
	registerNode(t, c, "desk")
	c.SetNameTaken(func(s string) bool { return s == "analytics" })
	events := make(chan NodeChange, 32)
	c.SetNodeObserver(func(e NodeChange) { events <- e })
	var gone []string
	c.SetGone(func(s string) { gone = append(gone, s) })
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	raw, _ := json.Marshal(tailcfg.MapRequest{NodeKey: k, Stream: true})
	done := make(chan struct{})
	go func() {
		defer close(done)
		c.serveMap(httptest.NewRecorder(), httptest.NewRequest("POST", "/machine/map", bytes.NewReader(raw)).WithContext(ctx), machine)
	}()
	expectNodeChange(t, events, "laptop", "online")
	old := "laptop"
	for _, tc := range []struct{ ask, want string }{{"work", "work"}, {"hub", "hub-2"}, {"desk", "desk-2"}, {"analytics", "analytics-2"}, {"Bad Name!", "bad-name"}} {
		if got, err := c.rename(machine, k, tc.ask); err != nil || got != tc.want {
			t.Fatalf("rename %q: %q %v, want %q", tc.ask, got, err, tc.want)
		}
		if len(gone) == 0 || gone[len(gone)-1] != old {
			t.Fatalf("rename %q answered before the hub dropped %q: %v", tc.ask, old, gone)
		}
		expectNodeChange(t, events, old, "removed")
		expectNodeChange(t, events, tc.want, "joined")
		expectNodeChange(t, events, tc.want, "online")
		old = tc.want
	}
	if got, err := c.rename(machine, k, "bad-name"); err != nil || got != "bad-name" || len(gone) != 5 || len(events) != 0 {
		t.Fatalf("same name: %q %v, gone %v, %d events", got, err, gone, len(events))
	}
	cancel()
	<-done
	names := []string{}
	for _, n := range c.Nodes() {
		names = append(names, n.Name)
	}
	if slices.Sort(names); !slices.Equal(names, []string{"bad-name", "desk", "hub"}) {
		t.Fatalf("nodes %v", names)
	}
}

// Anyone can open a control channel: only a member's own keys get the hub to list function prefixes.
func TestRenameRefusesStrangersBeforeHubWork(t *testing.T) {
	c := newCoordinator(t, t.TempDir())
	hubMachine, hubKey := registerNode(t, c, "hub")
	machine, k := registerNode(t, c, "laptop")
	other, otherKey := registerNode(t, c, "desk")
	work := 0
	c.SetBeforeRename(func() error { work++; return nil })
	for _, tc := range []struct {
		machine key.MachinePublic
		node    key.NodePublic
	}{{other, k}, {machine, otherKey}, {key.NewMachine().Public(), key.NewNode().Public()}, {hubMachine, hubKey}} {
		if got, err := c.rename(tc.machine, tc.node, "x"); err == nil {
			t.Fatalf("renamed to %q", got)
		}
	}
	if err := c.Remove("desk"); err != nil {
		t.Fatal(err)
	}
	if _, err := c.rename(other, otherKey, "x"); err == nil || work != 0 {
		t.Fatalf("removed node renamed (%v) or the hub worked %d times", err, work)
	}
	// Removed while the hub listed function prefixes: still refused.
	c.SetBeforeRename(func() error { return c.Remove("laptop") })
	if _, err := c.rename(machine, k, "x"); err == nil {
		t.Fatal("a node removed during the rename was renamed")
	}
}

// The creating token follows the name, or the hub refuses its own state at the next start.
func TestRenameKeepsStateValid(t *testing.T) {
	dir := t.TempDir()
	c := newCoordinator(t, dir)
	_, _, secret := issueToken(t, c, join.TokenOptions{})
	machine := key.NewMachine().Public()
	req := tokenRequest(secret, "runner")
	if !c.register(machine, "127.0.0.1", req).MachineAuthorized {
		t.Fatal("token join")
	}
	if got, err := c.rename(machine, req.NodeKey, "work"); err != nil || got != "work" {
		t.Fatalf("rename: %q %v", got, err)
	}
	file := c.file
	c.file = t.TempDir()
	if got, err := c.rename(machine, req.NodeKey, "again"); err == nil {
		t.Fatalf("rename with a failed save: %q", got)
	}
	c.file = file
	if n := c.Nodes(); len(n) != 1 || n[0].Name != "work" || !slices.Equal(c.st.Tokens[0].Nodes, []string{"work"}) {
		t.Fatalf("failed save kept %+v, token nodes %v", n, c.st.Tokens[0].Nodes)
	}
	if err := c.Close(); err != nil {
		t.Fatal(err)
	}
	if _, err := New(dir, nil); err != nil {
		t.Fatalf("renamed state does not load: %v", err)
	}
}
