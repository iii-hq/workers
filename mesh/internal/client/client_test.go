package client

import (
	"context"
	"crypto/ed25519"
	"encoding/json"
	"errors"
	"maps"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"reflect"
	"slices"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/coder/websocket"
	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/coordinator"
	"github.com/iii-hq/workers/mesh/internal/join"
	"github.com/iii-hq/workers/mesh/internal/meshnet"
	"tailscale.com/types/key"
)

func TestConfigValidate(t *testing.T) {
	ok := Config{HubURL: "http://hub:49180", Join: "secret://MESH_JOIN", Name: "harness-mbp", Share: []string{"mesh-probe", "ide"}}
	if err := ok.Validate(); err != nil {
		t.Fatal(err)
	}
	if err := (Config{Join: "env://MESH_JOIN"}).Validate(); err != nil {
		t.Fatal(err)
	}
	for _, bad := range []Config{
		{Join: "ABCD-EFGH"}, {Join: "iiimj_literal"},
		{Name: "Bad"}, {Name: "1node"},
		{Share: []string{"mesh"}}, {Share: []string{"secrets"}}, {Share: []string{"configuration"}}, {Share: []string{"rbac-proxy"}},
		{Share: []string{"../ide"}}, {Share: []string{""}},
	} {
		if bad.Validate() == nil {
			t.Fatalf("accepted %+v", bad)
		}
	}
}

func TestParseJoinTokenOrCode(t *testing.T) {
	pub, _, _ := ed25519.GenerateKey(nil)
	_, token, err := join.NewToken("https://hub.example:49180", pub, join.TokenOptions{})
	if err != nil {
		t.Fatal(err)
	}
	a, err := parseJoin("http://ignored:1", token)
	if err != nil || a.Token != token || a.PublicURL != "https://hub.example:49180" {
		t.Fatalf("token join %+v %v", a, err)
	}
	a, err = parseJoin("http://hub:49180", "ABCD-EFGH")
	if err != nil || a.Code != "ABCD-EFGH" || a.PublicURL != "http://hub:49180" {
		t.Fatalf("code join %+v %v", a, err)
	}
	if _, err = parseJoin("", "ABCD-EFGH"); err == nil {
		t.Fatal("a code without hub_url was accepted")
	}
}

func TestResolveJoin(t *testing.T) {
	local := &fakeLocal{fn: func(r iii.CallRequest) (json.RawMessage, error) {
		return json.RawMessage(`{"value":" ABCD-EFGH\n"}`), nil
	}}
	v, err := resolveJoin(context.Background(), local, "secret://MESH_JOIN")
	if err != nil || v != "ABCD-EFGH" {
		t.Fatalf("secret %q %v", v, err)
	}
	if c := local.called("secrets::resolve"); len(c) != 1 || string(c[0].Payload) != `{"ref":"secret://MESH_JOIN"}` {
		t.Fatalf("resolve call %+v", c)
	}
	t.Setenv("MESH_JOIN_TEST", "WXYZ-2345")
	if v, err = resolveJoin(context.Background(), local, "env://MESH_JOIN_TEST"); err != nil || v != "WXYZ-2345" {
		t.Fatalf("env %q %v", v, err)
	}
	if _, err = resolveJoin(context.Background(), local, "env://MESH_JOIN_UNSET_TEST"); err == nil {
		t.Fatal("unset env accepted")
	}
}

// Each /join counts against the hub's 10 per 10 minutes, so a value that may
// have spent one is tried once; a failure that reached no /join is retried.
func TestJoinTriesEachSpentValueOnce(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "mesh-client")
	attempts := 0
	r := &runner{dir: dir, bridge: NewBridge(&fakeLocal{}, "local"), enroll: func(_ context.Context, c Config, v string) (joinState, bool, error) {
		attempts++
		switch v {
		case "BAD0-CODE":
			return joinState{}, true, errors.New("wrong device code")
		case "DOWN-C0DE":
			return joinState{}, false, errors.New("connection refused")
		}
		return joinState{PublicURL: c.HubURL, Secret: "s", RequestedName: "laptop"}, true, nil
	}}
	c := Config{HubURL: "http://hub:49180", Join: "env://MESH_JOIN_TEST"}

	if st, _, err := r.join(context.Background(), Config{}); st != nil || err != nil || r.status().State != "waiting for join" {
		t.Fatalf("no join: %v %v %+v", st, err, r.status())
	}
	t.Setenv("MESH_JOIN_TEST", "BAD0-CODE")
	for range 3 {
		if st, _, _ := r.join(context.Background(), c); st != nil {
			t.Fatal("failed join returned a state")
		}
	}
	if s := r.status(); attempts != 1 || s.State != "join failed" || s.Error == "" {
		t.Fatalf("attempts %d, status %+v", attempts, s)
	}
	t.Setenv("MESH_JOIN_TEST", "DOWN-C0DE")
	for range 3 {
		r.join(context.Background(), c)
	}
	if s := r.status(); attempts != 4 || s.State != "waiting for join" || s.Error == "" {
		t.Fatalf("unspent failure: attempts %d, status %+v", attempts, s)
	}
	t.Setenv("MESH_JOIN_TEST", "GOOD-C0DE")
	st, fresh, err := r.join(context.Background(), c)
	if err != nil || st == nil || !fresh || attempts != 5 {
		t.Fatalf("changed value: %v %v %v attempts %d", st, fresh, err, attempts)
	}
	// The enrollment is kept pending before registration, so a restart reuses it instead of the code.
	if st, fresh, err = r.join(context.Background(), c); err != nil || st == nil || !fresh || !st.Pending || attempts != 5 {
		t.Fatalf("pending enrollment not reused: %+v %v %v attempts %d", st, fresh, err, attempts)
	}
	st.Pending = false
	if err = savePrivate(dir, "join.json", st); err != nil { // as start does once registered
		t.Fatal(err)
	}
	for path, mode := range map[string]os.FileMode{dir: 0700, filepath.Join(dir, "join.json"): 0600} {
		if fi, err := os.Stat(path); err != nil || fi.Mode().Perm() != mode {
			t.Fatalf("%s: %v %v", path, fi.Mode(), err)
		}
	}
	// Without the mesh state its machine key is new, so the hub must accept it again.
	if _, fresh, err = r.join(context.Background(), c); err != nil || !fresh || attempts != 5 {
		t.Fatalf("lost mesh state not fresh: %v %v attempts %d", fresh, err, attempts)
	}
	if err = os.MkdirAll(filepath.Join(dir, "mesh"), 0700); err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(filepath.Join(dir, "mesh", "state"), []byte("{}"), 0600); err != nil {
		t.Fatal(err)
	}
	if st, fresh, err = r.join(context.Background(), c); err != nil || st == nil || fresh || st.RequestedName != "laptop" || attempts != 5 {
		t.Fatalf("saved enrollment not reused: %+v %v attempts %d", st, err, attempts)
	}
}

// A missing hub_url or an unreachable hub spends no join attempt, so fixing it
// must not need a new join value.
func TestEnrollFailuresBeforeJoinAreRetried(t *testing.T) {
	pub, _, _ := ed25519.GenerateKey(nil)
	_, token, err := join.NewToken("http://127.0.0.1:1", pub, join.TokenOptions{})
	if err != nil {
		t.Fatal(err)
	}
	r := &runner{dir: t.TempDir(), bridge: NewBridge(&fakeLocal{}, "local"), enroll: enroll}
	// Only an unreachable hub is retried by itself: a value that does not parse waits for the save that fixes it.
	for _, c := range []struct {
		hubURL, value string
		retried       bool
	}{{"", "ABCD-EFGH", false}, {"", "iiimj_garbage", false}, {"http://127.0.0.1:1", "ABCD-EFGH", true}, {"", token, true}} {
		t.Setenv("MESH_JOIN_TEST", c.value)
		r.retry = false
		st, _, err := r.join(context.Background(), Config{HubURL: c.hubURL, Join: "env://MESH_JOIN_TEST"})
		if s := r.status(); st != nil || err == nil || s.State != "waiting for join" || r.tried != "" || r.retry != c.retried {
			t.Fatalf("%+v: %v, status %+v, tried %q, retry %v", c, err, s, r.tried, r.retry)
		}
	}
}

// The backend retries a refused secret forever and each refusal spends a hub
// join attempt, so a refused enrollment is dropped and its value not retried.
func TestRefusedRegistrationIsNotSavedOrRetried(t *testing.T) {
	var joins atomic.Int32
	hub, err := coordinator.New(t.TempDir(), func(secret, addr string) (string, error) {
		joins.Add(1)
		return "", errors.New("token revoked")
	})
	if err != nil {
		t.Fatal(err)
	}
	defer hub.Close()
	mux := http.NewServeMux()
	hub.Handle(mux)
	front := httptest.NewServer(mux)
	defer front.Close()
	attempts := 0
	r := &runner{o: Options{Version: "0.1.0"}, dir: t.TempDir(), bridge: NewBridge(&fakeLocal{}, "local"), enroll: func(context.Context, Config, string) (joinState, bool, error) {
		attempts++
		return joinState{PublicURL: front.URL, Secret: "iiimj_dead", Identity: join.Identity{Control: hub.Public()}, RequestedName: "laptop"}, false, nil
	}}
	t.Setenv("MESH_JOIN_TEST", "iiimj_dead")
	c := Config{Join: "env://MESH_JOIN_TEST"}
	r.start(t.Context(), c)
	if s := r.status(); r.done != nil || s.State != "join failed" || !strings.Contains(s.Error, "token revoked") {
		t.Fatalf("status %+v", s)
	}
	if _, err := os.Stat(filepath.Join(r.dir, "join.json")); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("refused enrollment saved: %v", err)
	}
	refusals := joins.Load()
	r.start(t.Context(), c)
	if attempts != 1 || joins.Load() != refusals || refusals > 2 {
		t.Fatalf("enrolled %d times, %d refusals then %d", attempts, refusals, joins.Load())
	}
}

// The hub replaces the code once it accepts a node, so a join stopped before
// join.json was final reconnects with its saved secret, and a lost mesh state
// is refused visibly instead of retried forever.
func TestInterruptedJoinReconnectsWithoutTheCode(t *testing.T) {
	var hub *coordinator.Coordinator
	hub, err := coordinator.New(t.TempDir(), func(secret, addr string) (string, error) {
		if secret != join.Secret(hub.Identity(), hub.Code()) {
			return "", join.ErrCode
		}
		return "", nil
	})
	if err != nil {
		t.Fatal(err)
	}
	defer hub.Close()
	mux := http.NewServeMux()
	hub.Handle(mux)
	front := httptest.NewServer(mux)
	defer front.Close()
	code := hub.Code()
	enrolled := joinState{PublicURL: front.URL, Secret: join.Secret(hub.Identity(), code), RequestedName: "laptop",
		Identity: join.Identity{Identity: hub.Identity().Public().(ed25519.PublicKey), Control: hub.Public()}}
	dir := t.TempDir()
	attempts := 0
	start := func(ctx context.Context) *runner {
		r := &runner{o: Options{Version: "0.1.0"}, dir: dir, bridge: NewBridge(&fakeLocal{}, "local"), enroll: func(context.Context, Config, string) (joinState, bool, error) {
			attempts++
			return enrolled, true, nil
		}}
		r.start(ctx, Config{Join: "env://MESH_JOIN_TEST"})
		return r
	}
	stop := func(r *runner) {
		r.cancel()
		r.stop(<-r.done)
	}
	t.Setenv("MESH_JOIN_TEST", code)
	// The worker stops right after the hub accepted the node and replaced the code.
	ctx, interrupt := context.WithCancel(t.Context())
	hub.SetNodeObserver(func(c coordinator.NodeChange) {
		if c.Event == "joined" {
			interrupt()
		}
	})
	r := start(ctx)
	next := hub.Code()
	if r.done != nil || next == code || len(hub.Nodes()) != 1 {
		t.Fatalf("interrupted join: %+v, code replaced %v, nodes %d", r.status(), next != code, len(hub.Nodes()))
	}
	if r = start(t.Context()); r.done == nil || attempts != 1 || hub.Code() != next || len(hub.Nodes()) != 1 {
		t.Fatalf("pending join: %+v, attempts %d, nodes %d", r.status(), attempts, len(hub.Nodes()))
	}
	stop(r)
	if st, fresh, err := r.join(t.Context(), Config{}); err != nil || st == nil || fresh {
		t.Fatalf("accepted join still pending: %+v %v %v", st, fresh, err)
	}
	if err = os.RemoveAll(filepath.Join(dir, "mesh")); err != nil {
		t.Fatal(err)
	}
	r = start(t.Context())
	if s := r.status(); r.done != nil || s.State != "join failed" || !strings.Contains(s.Error, join.ErrCode.Error()) {
		t.Fatalf("lost mesh state: %+v", s)
	}
	if _, err = os.Stat(filepath.Join(dir, "join.json")); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("refused enrollment kept: %v", err)
	}
}

// A build without a version would join, fail node.New and leave on every pass.
func TestStartRefusesMissingVersionBeforeJoining(t *testing.T) {
	attempts := 0
	r := &runner{dir: t.TempDir(), bridge: NewBridge(&fakeLocal{}, "local"), enroll: func(context.Context, Config, string) (joinState, bool, error) {
		attempts++
		return joinState{}, true, errors.New("unreachable")
	}}
	t.Setenv("MESH_JOIN_TEST", "ABCD-EFGH")
	r.start(context.Background(), Config{Join: "env://MESH_JOIN_TEST"})
	if s := r.status(); attempts != 0 || s.State != "error" || s.Error == "" {
		t.Fatalf("attempts %d, status %+v", attempts, s)
	}
}

// Security: a token names its hub by identity. A hub that answers at the
// token's URL with another identity must not be joined.
func TestEnrollPinsHubIdentity(t *testing.T) {
	hubPublic, hubKey, _ := ed25519.GenerateKey(nil)
	front := httptest.NewServer(join.IdentityHandler(hubKey, key.NewMachine().Public()))
	defer front.Close()
	other, _, _ := ed25519.GenerateKey(nil)
	_, forged, err := join.NewToken(front.URL, other, join.TokenOptions{})
	if err != nil {
		t.Fatal(err)
	}
	if st, spent, err := enroll(t.Context(), Config{Name: "laptop"}, forged); err == nil || spent || st.Secret != "" {
		t.Fatalf("joined a hub with another identity: %+v spent=%v err=%v", st, spent, err)
	}
	_, token, err := join.NewToken(front.URL, hubPublic, join.TokenOptions{})
	if err != nil {
		t.Fatal(err)
	}
	envelope, err := join.DecodeToken(token)
	if err != nil {
		t.Fatal(err)
	}
	st, spent, err := enroll(t.Context(), Config{Name: "laptop"}, token)
	if err != nil || spent || st.Secret != envelope.Secret || st.Secret == token || st.PublicURL != front.URL || !st.Identity.Identity.Equal(hubPublic) || st.RequestedName != "laptop" {
		t.Fatalf("token enrollment: %+v spent=%v err=%v", st, spent, err)
	}

	// A code learns the identity from /join. An /id that showed enroll another
	// identity (Redeem's own read sees the hub) must not be joined, and the
	// code counts as spent.
	_, impostorKey, _ := ed25519.GenerateKey(nil)
	impostor, genuine := join.IdentityHandler(impostorKey, key.NewMachine().Public()), join.IdentityHandler(hubKey, key.NewMachine().Public())
	var reads atomic.Int32
	code := join.NewCode()
	mux := http.NewServeMux()
	mux.Handle("/join", join.Handler(hubKey, func() string { return code }, join.NewLimit(10, 10*time.Minute, time.Now), nil))
	mux.HandleFunc("/id", func(w http.ResponseWriter, r *http.Request) {
		if reads.Add(1) == 1 {
			impostor.ServeHTTP(w, r)
			return
		}
		genuine.ServeHTTP(w, r)
	})
	codeFront := httptest.NewServer(mux)
	defer codeFront.Close()
	c := Config{Name: "laptop", HubURL: codeFront.URL}
	if st, spent, err := enroll(t.Context(), c, code); err == nil || !spent || st.Secret != "" {
		t.Fatalf("code joined a hub with another identity: %+v spent=%v err=%v", st, spent, err)
	}
	if st, spent, err = enroll(t.Context(), c, code); err != nil || !spent || st.Secret == "" || !st.Identity.Identity.Equal(hubPublic) {
		t.Fatalf("code enrollment: %+v spent=%v err=%v", st, spent, err)
	}
}

// A node the hub removed must not rejoin on the next pass.
func TestRemovedMarkerPreventsStart(t *testing.T) {
	dir := t.TempDir()
	if err := os.MkdirAll(filepath.Join(dir, "mesh"), 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "mesh", meshnet.Removed), nil, 0600); err != nil {
		t.Fatal(err)
	}
	attempts := 0
	r := &runner{o: Options{Version: "0.1.0"}, dir: dir, bridge: NewBridge(&fakeLocal{}, "local"), enroll: func(context.Context, Config, string) (joinState, bool, error) {
		attempts++
		return joinState{}, true, errors.New("unreachable")
	}}
	t.Setenv("MESH_JOIN_TEST", "ABCD-EFGH")
	r.start(t.Context(), Config{Join: "env://MESH_JOIN_TEST"})
	if s := r.status(); attempts != 0 || r.done != nil || s.State != removedState {
		t.Fatalf("attempts %d, status %+v", attempts, s)
	}
}

// A leave logs out, so the hub drops the machine, and the same join value
// never rejoins by itself: a reusable token would, on the next pass or boot.
func TestLeaveLogsOutForgetsAndStaysOut(t *testing.T) {
	hub, err := coordinator.New(t.TempDir(), func(secret, addr string) (string, error) { return "", nil })
	if err != nil {
		t.Fatal(err)
	}
	defer hub.Close()
	mux := http.NewServeMux()
	hub.Handle(mux)
	front := httptest.NewServer(mux)
	defer front.Close()
	enrolled := joinState{PublicURL: front.URL, Secret: "reusable", RequestedName: "laptop",
		Identity: join.Identity{Identity: hub.Identity().Public().(ed25519.PublicKey), Control: hub.Public()}}
	dir := t.TempDir()
	attempts := 0
	boot := func() *runner {
		return &runner{o: Options{Version: "0.1.0"}, dir: dir, bridge: NewBridge(&fakeLocal{}, "local"), enroll: func(context.Context, Config, string) (joinState, bool, error) {
			attempts++
			return enrolled, false, nil
		}}
	}
	gone := func(name string) bool {
		_, err := os.Stat(filepath.Join(dir, name))
		return errors.Is(err, os.ErrNotExist)
	}
	c := Config{Join: "env://MESH_JOIN_TEST"}
	t.Setenv("MESH_JOIN_TEST", "iiimj_reusable")
	r := boot()
	r.start(t.Context(), c)
	if r.done == nil || len(hub.Nodes()) != 1 {
		t.Fatalf("join: %+v, nodes %d", r.status(), len(hub.Nodes()))
	}
	first := hub.Nodes()[0].Addr
	res, err := r.leave(t.Context())
	if err != nil || !res.HubNotified || res.Warning != "" {
		t.Fatalf("leave: %+v %v", res, err)
	}
	if s := r.status(); r.done != nil || s.State != leftState || len(hub.Nodes()) != 0 || !gone("join.json") || !gone("mesh") {
		t.Fatalf("after leave: %+v, nodes %d", s, len(hub.Nodes()))
	}
	if b, _ := os.ReadFile(filepath.Join(dir, leftMarker)); string(b) != `{"join_digest":"`+joinDigest("iiimj_reusable")+`"}` {
		t.Fatalf("left marker %s", b)
	}
	// The next pass and the next boot keep out.
	for _, r = range []*runner{r, boot()} {
		r.start(t.Context(), c)
		if s := r.status(); r.done != nil || s.State != leftState || attempts != 1 || len(hub.Nodes()) != 0 {
			t.Fatalf("rejoined with the same value: %+v, attempts %d, nodes %d", s, attempts, len(hub.Nodes()))
		}
	}
	// Another value joins, as a new machine.
	t.Setenv("MESH_JOIN_TEST", "iiimj_other")
	r.start(t.Context(), c)
	if r.done == nil || attempts != 2 || len(hub.Nodes()) != 1 || hub.Nodes()[0].Addr == first || !gone(leftMarker) {
		t.Fatalf("new value: %+v, attempts %d, nodes %+v", r.status(), attempts, hub.Nodes())
	}
	// A hub that cannot be told still loses the machine locally, and the answer says so.
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	res, err = r.leave(ctx)
	if err != nil || res.HubNotified || !strings.Contains(res.Warning, "until it is removed there") {
		t.Fatalf("leave without the hub: %+v %v", res, err)
	}
	if s := r.status(); r.done != nil || s.State != leftState || s.Error != res.Warning || len(hub.Nodes()) != 1 || !gone("join.json") || !gone("mesh") {
		t.Fatalf("after leave without the hub: %+v, nodes %d", s, len(hub.Nodes()))
	}
	// The warning outlives a restart, so a reloaded page still shows it.
	r = boot()
	r.start(t.Context(), c)
	if s := r.status(); s.State != leftState || s.Error != res.Warning {
		t.Fatalf("warning after a restart: %+v", s)
	}
}

// A machine the hub removed while it was offline hears it only as a refused
// registration: it must still park as removed, and its leave owes no warning.
// Nor does a leave that reaches the hub just after a removal.
func TestRemovalReachesAnOfflineOrLeavingMachine(t *testing.T) {
	hub, err := coordinator.New(t.TempDir(), func(secret, addr string) (string, error) { return "", nil })
	if err != nil {
		t.Fatal(err)
	}
	defer hub.Close()
	mux := http.NewServeMux()
	hub.Handle(mux)
	front := httptest.NewServer(mux)
	defer front.Close()
	enrolled := joinState{PublicURL: front.URL, Secret: "reusable", RequestedName: "laptop", JoinDigest: joinDigest("iiimj_reusable"),
		Identity: join.Identity{Identity: hub.Identity().Public().(ed25519.PublicKey), Control: hub.Public()}}
	t.Setenv("MESH_JOIN_TEST", "iiimj_reusable")
	c := Config{Join: "env://MESH_JOIN_TEST"}
	run := func(dir string) *runner {
		r := &runner{o: Options{Version: "0.1.0"}, dir: dir, bridge: NewBridge(&fakeLocal{}, "local"), enroll: func(context.Context, Config, string) (joinState, bool, error) {
			return enrolled, false, nil
		}}
		r.start(t.Context(), c)
		if r.done == nil {
			t.Fatalf("join: %+v", r.status())
		}
		return r
	}
	removeOnly := func() {
		nodes := hub.Nodes()
		if len(nodes) != 1 {
			t.Fatalf("nodes %+v", nodes)
		}
		if err := hub.Remove(nodes[0].Name); err != nil {
			t.Fatal(err)
		}
	}

	dir := t.TempDir()
	r := run(dir)
	r.cancel()
	r.stop(<-r.done)
	removeOnly()
	r = run(dir)
	select {
	case err := <-r.done:
		r.stop(err)
	case <-time.After(20 * time.Second):
		t.Fatalf("still running after an offline removal: %+v", r.status())
	}
	if s := r.status(); s.State != removedState {
		t.Fatalf("offline removal: %+v", s)
	}
	if res, err := r.leave(t.Context()); err != nil || res.Warning != "" {
		t.Fatalf("leave after an offline removal: %+v %v", res, err)
	}

	r = run(t.TempDir())
	removeOnly()
	if res, err := r.leave(t.Context()); err != nil || res.Warning != "" {
		t.Fatalf("leave right after a removal: %+v %v", res, err)
	}
}

// A leave cut short after its marker keeps out, finishes forgetting and says
// the hub may still list the machine.
func TestInterruptedLeaveFinishesForgetting(t *testing.T) {
	dir := t.TempDir()
	if err := savePrivate(dir, "join.json", joinState{RequestedName: "laptop", JoinDigest: joinDigest("iiimj_reusable")}); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(filepath.Join(dir, "mesh"), 0700); err != nil {
		t.Fatal(err)
	}
	r := &runner{o: Options{Version: "0.1.0"}, dir: dir, bridge: NewBridge(&fakeLocal{}, "local"), enroll: func(context.Context, Config, string) (joinState, bool, error) {
		t.Fatal("rejoined with the value left behind")
		return joinState{}, false, nil
	}}
	// What a leave writes before it logs out.
	if err := savePrivate(dir, leftMarker, leftNote{JoinDigest: joinDigest("iiimj_reusable"), Warning: "the hub may still list this machine"}); err != nil {
		t.Fatal(err)
	}
	t.Setenv("MESH_JOIN_TEST", "iiimj_reusable")
	r.start(t.Context(), Config{Join: "env://MESH_JOIN_TEST"})
	if s := r.status(); s.State != leftState || s.Error != "the hub may still list this machine" {
		t.Fatalf("interrupted leave: %+v", s)
	}
	for _, name := range []string{"join.json", "mesh"} {
		if _, err := os.Stat(filepath.Join(dir, name)); !errors.Is(err, os.ErrNotExist) {
			t.Fatalf("%s kept: %v", name, err)
		}
	}
}

// What a leave remembers: the value the enrollment was made with, or, for an
// enrollment that does not name it, the first value read afterwards, unless
// the hub removed the machine, which already made every old value useless.
func TestLeftClientJoinsOnlyWithAnotherValue(t *testing.T) {
	dir := t.TempDir()
	attempts := 0
	boot := func() *runner {
		return &runner{o: Options{Version: "0.1.0"}, dir: dir, bridge: NewBridge(&fakeLocal{}, "local"), enroll: func(context.Context, Config, string) (joinState, bool, error) {
			attempts++
			return joinState{}, false, errors.New("connection refused")
		}}
	}
	// enrolled saves what a join with value would have, plus the hub's removal.
	enrolled := func(value string, removed bool) {
		st := joinState{RequestedName: "laptop"}
		if value != "" {
			st.JoinDigest = joinDigest(value)
		}
		if err := savePrivate(dir, "join.json", st); err != nil {
			t.Fatal(err)
		}
		if err := os.MkdirAll(filepath.Join(dir, "mesh"), 0700); err != nil {
			t.Fatal(err)
		}
		if !removed {
			return
		}
		if err := os.WriteFile(filepath.Join(dir, "mesh", meshnet.Removed), nil, 0600); err != nil {
			t.Fatal(err)
		}
	}
	c := Config{Join: "env://MESH_JOIN_TEST"}
	state := func(r *runner, value string) Status {
		t.Setenv("MESH_JOIN_TEST", value)
		r.start(t.Context(), c)
		return r.status()
	}
	r := boot()
	if _, err := r.leave(t.Context()); err == nil {
		t.Fatal("left without an enrollment")
	}
	if _, err := os.Stat(filepath.Join(dir, leftMarker)); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("a refused leave kept a marker: %v", err)
	}

	// The hub removed the machine: the leave forgets it without a warning.
	enrolled("AAAA-AAAA", true)
	if s := state(r, "AAAA-AAAA"); s.State != removedState || s.Error == "" {
		t.Fatalf("removed: %+v", s)
	}
	if res, err := r.leave(t.Context()); err != nil || res.HubNotified || res.Warning != "" {
		t.Fatalf("leave after removal: %+v %v", res, err)
	}
	if s := state(r, "AAAA-AAAA"); s.State != leftState || s.Error != "" || attempts != 0 {
		t.Fatalf("same value: %+v, attempts %d", s, attempts)
	}
	if s := state(boot(), "AAAA-AAAA"); s.State != leftState || attempts != 0 {
		t.Fatalf("same value after a restart: %+v, attempts %d", s, attempts)
	}
	// The new code may be stored before the leave: it is not the value left behind.
	if s := state(r, "BBBB-BBBB"); s.State != "waiting for join" || attempts != 1 {
		t.Fatalf("new value: %+v, attempts %d", s, attempts)
	}

	// An enrollment that does not name its value, still listed by the hub: an
	// unreadable value stays out, and the first one read counts as the old one.
	enrolled("", false)
	res, err := r.leave(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if s := state(r, ""); s.State != leftState || !strings.Contains(s.Error, res.Warning) || !strings.Contains(s.Error, "resolve") || attempts != 1 {
		t.Fatalf("unreadable value: %+v, attempts %d", s, attempts)
	}
	if s := state(r, "CCCC-CCCC"); s.State != leftState || s.Error != res.Warning || attempts != 1 {
		t.Fatalf("first value read: %+v, attempts %d", s, attempts)
	}
	if s := state(r, "DDDD-DDDD"); s.State != "waiting for join" || attempts != 2 {
		t.Fatalf("second value: %+v, attempts %d", s, attempts)
	}

	// The same, removed by the hub: a new code stored before the leave joins at once.
	enrolled("", true)
	if s := state(r, "EEEE-EEEE"); s.State != removedState || attempts != 2 {
		t.Fatalf("removed: %+v, attempts %d", s, attempts)
	}
	if _, err := r.leave(t.Context()); err != nil {
		t.Fatal(err)
	}
	if s := state(r, "EEEE-EEEE"); s.State != "waiting for join" || attempts != 3 {
		t.Fatalf("new code after a removal: %+v, attempts %d", s, attempts)
	}

	// No join configured at the leave: it stays out until a value is set.
	enrolled("", true)
	if _, err := r.leave(t.Context()); err != nil {
		t.Fatal(err)
	}
	r.start(t.Context(), Config{})
	if s := r.status(); s.State != leftState || attempts != 3 {
		t.Fatalf("no join value: %+v, attempts %d", s, attempts)
	}
	if s := state(r, "DDDD-DDDD"); s.State != "waiting for join" || attempts != 4 {
		t.Fatalf("value set after the leave: %+v, attempts %d", s, attempts)
	}

	// A refused registration kept only the mesh state the hub removed: it can still leave.
	if err := os.MkdirAll(filepath.Join(dir, "mesh"), 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "mesh", meshnet.Removed), nil, 0600); err != nil {
		t.Fatal(err)
	}
	if res, err := r.leave(t.Context()); err != nil || res.Warning != "" {
		t.Fatalf("leave without join.json: %+v %v", res, err)
	}
	if _, err := os.Stat(filepath.Join(dir, "mesh")); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("mesh state kept: %v", err)
	}
}

// mesh::leave is served by the loop that owns the node, before its next pass:
// a pass can take a minute, and an event is waiting by then.
func TestLeaveRequestIsServedByTheLoop(t *testing.T) {
	t.Setenv("MESH_JOIN_TEST", "ABCD-EFGH")
	// select picks among ready cases at random, so a few rounds catch a pass that jumps the queue.
	for range 10 {
		dir := t.TempDir()
		if err := savePrivate(dir, "join.json", joinState{RequestedName: "laptop", JoinDigest: joinDigest("ABCD-EFGH")}); err != nil {
			t.Fatal(err)
		}
		var attempts, fetches atomic.Int32
		var jumped atomic.Bool
		type answer struct {
			res LeaveResult
			err error
		}
		left := make(chan answer, 1)
		var r *runner
		r = &runner{dir: dir, bridge: NewBridge(&fakeLocal{}, "local"), leaving: make(chan chan leaveReply), state: "starting", wake: make(chan struct{}, 1), dirty: make(chan struct{}, 1),
			o: Options{Version: "0.1.0", Fetch: func(ctx context.Context) (string, *Config, error) {
				if fetches.Add(1) == 1 {
					go func() {
						res, err := r.requestLeave(ctx)
						left <- answer{res, err}
					}()
					time.Sleep(100 * time.Millisecond) // the leave waits behind this pass
					r.kick(wakeApply)                  // and so does an event
					return "", nil, errors.New("slow pass")
				}
				if _, err := os.Stat(filepath.Join(dir, "join.json")); err == nil {
					jumped.Store(true)
					return "", nil, errors.New("a pass went before the queued leave")
				}
				return "client", &Config{Join: "env://MESH_JOIN_TEST"}, nil
			}},
			enroll: func(context.Context, Config, string) (joinState, bool, error) {
				attempts.Add(1)
				return joinState{}, false, errors.New("connection refused")
			}}
		ctx, cancel := context.WithCancel(t.Context())
		stopped := make(chan struct{})
		go func() { defer close(stopped); r.loop(ctx) }()
		a := <-left
		if a.err != nil || a.res.HubNotified || a.res.Warning == "" {
			t.Fatalf("leave: %+v %v", a.res, a.err)
		}
		// Later passes resolve the same join value and keep out.
		for n := fetches.Load(); fetches.Load() < n+3; time.Sleep(time.Millisecond) {
			r.kick(wakeApply)
		}
		if _, err := r.requestLeave(ctx); err == nil {
			t.Fatal("left twice")
		}
		cancel()
		<-stopped
		if s := r.status(); jumped.Load() || attempts.Load() != 0 || s.State != leftState || s.Error != a.res.Warning {
			t.Fatalf("jumped %v, attempts %d, status %+v", jumped.Load(), attempts.Load(), s)
		}
	}
}

// watched runs r.loop and returns what reads the statuses it publishes.
func watched(t *testing.T, r *runner) (next func(what string, ok func(Status) bool) Status, stop func()) {
	t.Helper()
	r.leaving, r.wake, r.dirty, r.state = make(chan chan leaveReply), make(chan struct{}, 1), make(chan struct{}, 1), "starting"
	r.bridge.Changed = r.changed
	published := make(chan Status, 256)
	r.emit = func(b []byte) {
		var s Status
		if err := json.Unmarshal(b, &s); err != nil {
			t.Error(err)
		}
		published <- s
	}
	ctx, cancel := context.WithCancel(t.Context())
	done := make(chan struct{})
	go func() { defer close(done); r.loop(ctx) }()
	var seen []string
	return func(what string, ok func(Status) bool) Status {
			t.Helper()
			for {
				select {
				case s := <-published:
					seen = append(seen, s.State)
					if ok(s) {
						return s
					}
				case <-time.After(20 * time.Second):
					t.Fatalf("no status with %s; states so far %v", what, seen)
				}
			}
		}, func() {
			cancel()
			<-done
		}
}

// testHub serves a real coordinator's control plane; enrolled joins it as name.
func testHub(t *testing.T) (*coordinator.Coordinator, *httptest.Server, func(name string) joinState) {
	t.Helper()
	hub, err := coordinator.New(t.TempDir(), func(secret, addr string) (string, error) { return "", nil })
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = hub.Close() })
	mux := http.NewServeMux()
	hub.Handle(mux)
	front := httptest.NewServer(mux)
	t.Cleanup(front.Close)
	return hub, front, func(name string) joinState {
		return joinState{PublicURL: front.URL, Secret: "reusable", RequestedName: name,
			Identity: join.Identity{Identity: hub.Identity().Public().(ed25519.PublicKey), Control: hub.Public()}}
	}
}

// Every change is applied on its event: the retry timer is an hour, so nothing here waits on a clock.
func TestEventsApplyWithoutATick(t *testing.T) {
	defer func(a, b time.Duration) { retryMin, retryMax = a, b }(retryMin, retryMax)
	retryMin, retryMax = time.Hour, time.Hour
	t.Setenv("HOME", t.TempDir())
	t.Setenv("MESH_JOIN_TEST", "iiimj_reusable")
	hub, _, enrolled := testHub(t)
	var mu sync.Mutex
	rows := []map[string]any{row("probe::echo", "local", "probe")}
	local := engine(&rows)
	list := local.fn
	local.fn = func(r iii.CallRequest) (json.RawMessage, error) {
		mu.Lock()
		defer mu.Unlock()
		return list(r)
	}
	cfg := Config{Join: "env://MESH_JOIN_TEST", Name: "laptop"}
	r := &runner{dir: t.TempDir(), bridge: NewBridge(local, "local"), enroll: func(context.Context, Config, string) (joinState, bool, error) {
		return enrolled("laptop"), true, nil
	}, o: Options{Version: "0.1.0", Fetch: func(context.Context) (string, *Config, error) {
		mu.Lock()
		defer mu.Unlock()
		c := cfg
		c.Share = slices.Clone(cfg.Share)
		return "client", &c, nil
	}}}
	next, stop := watched(t, r)
	defer stop()
	next("joining", func(s Status) bool { return s.State == "joining" })
	next("waiting for hub", func(s Status) bool { return s.State == "waiting for hub" })
	next("connected", func(s Status) bool { return s.State == "connected" && s.Node == "laptop" })

	mu.Lock()
	cfg.Share = []string{"probe"}
	mu.Unlock()
	r.kick(wakeApply) // what a configuration::on-updated event does
	next("probe shared", func(s Status) bool {
		return len(s.Shared) == 1 && slices.Equal(s.Shared[0].Functions, []string{"laptop::probe::echo"})
	})

	mu.Lock()
	rows = append(rows, row("probe::more", "local", "probe"))
	mu.Unlock()
	r.kick(wakeSync) // what an engine::on-functions-changed event does
	next("the new function", func(s Status) bool { return len(s.Shared) == 1 && len(s.Shared[0].Functions) == 2 })

	mu.Lock()
	cfg.Name = "work"
	mu.Unlock()
	r.kick(wakeApply)
	next("renaming", func(s Status) bool { return s.State == "renaming" })
	s := next("renamed", func(s Status) bool { return s.State == "connected" && s.Node == "work" })
	if s.RequestedName != "work" || s.RenameError != "" || !slices.Equal(s.Shared[0].Functions, []string{"work::probe::echo", "work::probe::more"}) {
		t.Fatalf("renamed status %+v", s)
	}
	if n := hub.Nodes(); len(n) != 1 || n[0].Name != "work" {
		t.Fatalf("hub nodes %+v", n)
	}

	// Another join value or hub URL is used only after a leave, and the status says so.
	t.Setenv("MESH_JOIN_TEST", "iiimj_other")
	r.kick(wakeApply) // what a secrets::on-changed event does
	next("another join value", func(s Status) bool { return s.JoinUnused && s.State == "connected" && s.Node == "work" })
	t.Setenv("MESH_JOIN_TEST", "iiimj_reusable")
	r.kick(wakeApply)
	next("the join value in use", func(s Status) bool { return !s.JoinUnused })
	mu.Lock()
	cfg.HubURL = "https://other.example.com"
	mu.Unlock()
	r.kick(wakeApply)
	next("another hub URL", func(s Status) bool { return s.JoinUnused && s.State == "connected" })

	if err := hub.Remove("work"); err != nil {
		t.Fatal(err)
	}
	next("removed", func(s Status) bool { return s.State == removedState })
}

// A failure no event announces the end of is retried later and later; other events neither retry it nor push it back.
func TestRetryBacksOffAndEventsDoNotPushItBack(t *testing.T) {
	defer func(a, b time.Duration) { retryMin, retryMax = a, b }(retryMin, retryMax)
	retryMin, retryMax = 40*time.Millisecond, 160*time.Millisecond
	var fetches atomic.Int32
	var times []time.Time
	r := &runner{dir: t.TempDir(), bridge: NewBridge(&fakeLocal{}, "local"), o: Options{Version: "0.1.0", Fetch: func(context.Context) (string, *Config, error) {
		times = append(times, time.Now())
		if fetches.Add(1) < 5 {
			return "", nil, errors.New("engine unavailable")
		}
		return "client", &Config{}, nil
	}}}
	next, stop := watched(t, r)
	defer stop()
	churn, quiet := make(chan struct{}), make(chan struct{})
	go func() {
		defer close(quiet)
		for {
			select {
			case <-churn:
				return
			case <-time.After(5 * time.Millisecond):
				r.kick(wakeSync)
				r.kick(wakeNode)
			}
		}
	}()
	s := next("a retried error", func(s Status) bool { return s.State == "error" && s.Retrying })
	if !strings.Contains(s.Error, "read configuration: engine unavailable") {
		t.Fatalf("status %+v", s)
	}
	next("settled", func(s Status) bool { return s.State == "waiting for join" && !s.Retrying && s.Error == "" })
	close(churn)
	<-quiet
	var gaps []time.Duration
	for i := 1; i < len(times); i++ {
		gaps = append(gaps, times[i].Sub(times[i-1]))
	}
	if len(gaps) != 4 || gaps[0] < retryMin || gaps[1] < 2*retryMin || gaps[2] < retryMax || gaps[3] < retryMax || gaps[3] >= 2*retryMax {
		t.Fatalf("gaps between passes %v", gaps)
	}
	time.Sleep(3 * retryMax)
	if n := fetches.Load(); n != 5 {
		t.Fatalf("%d passes with nothing to retry", n)
	}
}

// A node that stops with an error starts again only after a growing delay, and the status says the worker keeps trying.
func TestFailedNodeRestartsAfterABackoff(t *testing.T) {
	defer func(a, b time.Duration) { retryMin, retryMax = a, b }(retryMin, retryMax)
	retryMin, retryMax = 100*time.Millisecond, time.Minute
	t.Setenv("MESH_JOIN_TEST", "iiimj_reusable")
	_, _, enrolled := testHub(t)
	r := &runner{dir: t.TempDir(), bridge: NewBridge(&fakeLocal{}, "local"), enroll: func(context.Context, Config, string) (joinState, bool, error) {
		return enrolled("laptop"), true, nil
	}, o: Options{Version: "0.1.0", Fetch: func(context.Context) (string, *Config, error) {
		return "client", &Config{Join: "env://MESH_JOIN_TEST"}, nil
	}}}
	next, stop := watched(t, r)
	defer stop()
	connected := func(s Status) bool { return s.State == "connected" }
	next("connected", connected)
	var gaps []time.Duration
	for range 3 {
		// What node.Run returns when the hub engine rejects the node's session, for example a stale twin holding its worker name.
		r.done <- errors.New("connect node service: worker name taken")
		failed := time.Now()
		next("a retried error", func(s Status) bool {
			return s.State == "error" && s.Retrying && strings.Contains(s.Error, "worker name taken")
		})
		next("connected again", connected)
		gaps = append(gaps, time.Since(failed))
	}
	if gaps[0] < retryMin || gaps[1] < 2*retryMin || gaps[2] < 4*retryMin {
		t.Fatalf("started again after %v", gaps)
	}
}

// A rename that fails keeps the old name, says why and is asked again after the backoff, on the hub's side or not.
func TestRenameFailureKeepsTheNameAndIsAskedAgain(t *testing.T) {
	defer func(a, b time.Duration) { retryMin, retryMax = a, b }(retryMin, retryMax)
	retryMin, retryMax = 20*time.Millisecond, 80*time.Millisecond
	t.Setenv("MESH_JOIN_TEST", "iiimj_reusable")
	hub, front, enrolled := testHub(t)
	var asks atomic.Int32
	hub.SetBeforeRename(func() error { asks.Add(1); return errors.New("list engine function prefixes: engine unavailable") })
	var name atomic.Value
	name.Store("laptop")
	dir := t.TempDir()
	r := &runner{dir: dir, bridge: NewBridge(&fakeLocal{}, "local"), enroll: func(context.Context, Config, string) (joinState, bool, error) {
		return enrolled("laptop"), true, nil
	}, o: Options{Version: "0.1.0", Fetch: func(context.Context) (string, *Config, error) {
		return "client", &Config{Join: "env://MESH_JOIN_TEST", Name: name.Load().(string)}, nil
	}}}
	requested := func() string {
		t.Helper()
		var st joinState
		b, err := os.ReadFile(filepath.Join(dir, "join.json"))
		if err == nil {
			err = json.Unmarshal(b, &st)
		}
		if err != nil {
			t.Fatal(err)
		}
		return st.RequestedName
	}
	next, stop := watched(t, r)
	defer stop()
	next("connected", func(s Status) bool { return s.State == "connected" && s.Node == "laptop" })
	name.Store("work")
	r.kick(wakeApply)
	s := next("refused", func(s Status) bool { return s.RenameError != "" && s.Retrying })
	// The failure stays shown while the name is asked again (renaming).
	if s.State != "connected" && s.State != "renaming" || s.Node != "laptop" || s.RequestedName != "laptop" || !strings.Contains(s.RenameError, "engine unavailable") || !s.Retrying || requested() != "laptop" {
		t.Fatalf("status %+v, join.json asks %q", s, requested())
	}
	// The hub's engine may be back a second later: the same name is asked again by itself.
	for deadline := time.Now().Add(5 * time.Second); asks.Load() < 3; time.Sleep(5 * time.Millisecond) {
		if time.Now().After(deadline) {
			t.Fatalf("asked %d times", asks.Load())
		}
	}
	if hub.Nodes()[0].Name != "laptop" {
		t.Fatalf("nodes %+v", hub.Nodes())
	}
	// No name to ask for: the failure is gone and nothing is asked again.
	name.Store("")
	r.kick(wakeApply)
	next("no rename to ask", func(s Status) bool { return s.State == "connected" && s.RenameError == "" && !s.Retrying })
	asked := asks.Load()
	time.Sleep(3 * retryMax)
	if asks.Load() != asked {
		t.Fatalf("asked %d more times with no name", asks.Load()-asked)
	}

	hub.SetBeforeRename(nil)
	name.Store("desk")
	r.kick(wakeApply)
	next("renamed", func(s Status) bool {
		return s.State == "connected" && s.Node == "desk" && s.RequestedName == "desk" && s.RenameError == ""
	})
	if requested() != "desk" {
		t.Fatalf("join.json asks %q", requested())
	}

	_ = hub.Close()
	front.Close()
	name.Store("lab")
	r.kick(wakeApply)
	s = next("hub not reached", func(s Status) bool { return s.RenameError != "" && s.Retrying })
	if !strings.HasPrefix(s.RenameError, "hub not reached") || !s.Retrying || s.Node != "desk" || requested() != "desk" {
		t.Fatalf("status %+v", s)
	}
}

// Run observes its three sources, answers their events and gives a new status observer the current status.
func TestRunObservesItsEventSources(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	ctx, cancel := context.WithTimeout(t.Context(), 20*time.Second)
	defer cancel()
	frames := make(chan map[string]json.RawMessage, 256)
	send := make(chan []byte, 16)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer conn.CloseNow()
		greet, _ := json.Marshal(map[string]any{"type": "workerregistered", "worker_id": "wire", "features": []string{"observe.v1", "observe.v2", "local-dispatch.v1"}})
		_ = conn.Write(ctx, websocket.MessageText, greet)
		go func() {
			for b := range send {
				_ = conn.Write(ctx, websocket.MessageText, b)
			}
		}()
		for {
			_, b, err := conn.Read(ctx)
			if err != nil {
				return
			}
			var frame map[string]json.RawMessage
			if json.Unmarshal(b, &frame) != nil {
				return
			}
			frames <- frame
			var answer map[string]any
			switch string(frame["type"]) {
			case `"registerfunction"`:
				answer = map[string]any{"type": "functionregistered", "id": frame["id"], "namespace": "project", "revision": 0, "local_dispatch": true, "observers": []any{}}
			case `"observe"`:
				answer = map[string]any{"type": "observeresult", "id": frame["id"], "state": "active"}
			case `"invokefunction"`:
				if len(frame["invocation_id"]) > 0 {
					answer = map[string]any{"type": "invocationresult", "invocation_id": frame["invocation_id"], "function_id": frame["function_id"], "result": map[string]any{}}
				}
			}
			if answer != nil {
				out, _ := json.Marshal(answer)
				send <- out
			}
		}
	}))
	defer server.Close()
	client := iii.New("ws"+strings.TrimPrefix(server.URL, "http"), iii.WithOtel(iii.OtelConfig{Enabled: new(bool)}), iii.WithName("mesh"), iii.WithNamespace("project"))
	defer client.Close()
	if err := client.Connect(ctx); err != nil {
		t.Fatal(err)
	}
	var fetches atomic.Int32
	done := make(chan error, 1)
	go func() {
		done <- Run(ctx, Options{Local: client, Namespace: "project", Version: "0.1.0", Dir: t.TempDir(), ConfigID: "mesh-lab", Fetch: func(context.Context) (string, *Config, error) {
			fetches.Add(1)
			return "client", &Config{Share: []string{"probe"}}, nil
		}})
	}()
	type endpoint struct {
		FunctionID string         `json:"function_id"`
		Namespace  string         `json:"namespace"`
		Filters    map[string]any `json:"filters"`
		Deliver    string         `json:"deliver"`
		Args       any            `json:"args"`
	}
	registered := map[string]map[string]json.RawMessage{}
	observed := map[string][2]endpoint{}
	observers := map[string]string{} // observer id by target
	lists := 0
	read := func(what string, until func() bool) {
		t.Helper()
		for !until() {
			select {
			case f := <-frames:
				switch string(f["type"]) {
				case `"registerfunction"`:
					var id string
					_ = json.Unmarshal(f["id"], &id)
					registered[id] = f
				case `"observe"`:
					var source, target endpoint
					var id string
					_ = json.Unmarshal(f["source"], &source)
					_ = json.Unmarshal(f["target"], &target)
					_ = json.Unmarshal(f["id"], &id)
					observed[source.FunctionID] = [2]endpoint{source, target}
					observers[source.FunctionID] = id
				case `"invokefunction"`:
					if string(f["function_id"]) == `"engine::functions::list"` {
						lists++
					}
				case `"invocationsobserved"`:
					var out struct {
						Items []struct {
							FunctionID string `json:"function_id"`
							Matched    []string
							Input      Status
						}
					}
					_ = json.Unmarshal(mustJSON(t, f), &out)
					for _, item := range out.Items {
						if item.FunctionID == StatusChanged && slices.Contains(item.Matched, "page-1") && item.Input.Mode == "client" {
							registered["delivered"] = f
						}
					}
				}
			case err := <-done:
				t.Fatalf("run ended: %v", err)
			case <-ctx.Done():
				t.Fatalf("%s: registered %v observed %v", what, slices.Collect(maps.Keys(registered)), observed)
			}
		}
	}
	read("registrations and observers", func() bool { return len(observed) == 3 && registered[StatusChanged] != nil && lists > 0 })
	status := registered[StatusChanged]
	if string(status["private"]) != "true" || !strings.Contains(string(status["metadata"]), `"internal":true`) || len(status["args_format"]) > 0 && string(status["args_format"]) != "null" {
		t.Fatalf("status events registered as %v", status)
	}
	in := []any{"configuration:registered", "configuration:updated", "configuration:deleted"}
	for id, want := range map[string][2]endpoint{
		"configuration::on-updated": {{FunctionID: "configuration::on-updated", Namespace: "default", Deliver: "output", Filters: map[string]any{"input.id": map[string]any{"$eq": "mesh-lab"}, "input.event_type": map[string]any{"$in": in}}},
			{FunctionID: "mesh::on-config-change", Namespace: "project"}},
		"secrets::on-changed":          {{FunctionID: "secrets::on-changed", Namespace: "project", Deliver: "output"}, {FunctionID: "mesh::on-config-change", Namespace: "project"}},
		"engine::on-functions-changed": {{FunctionID: "engine::on-functions-changed", Namespace: "default", Deliver: "output"}, {FunctionID: "mesh::on-functions-change", Namespace: "project"}},
	} {
		if got := observed[id]; !reflect.DeepEqual(got, want) {
			t.Fatalf("observe %s:\n got %+v\nwant %+v", id, got, want)
		}
	}
	activate := func(source string) {
		target := observed[source][1]
		out, _ := json.Marshal(map[string]any{"type": "invokefunction", "invocation_id": "7b0c7c8e-6c6a-4f0e-9d5e-0f1e2d3c4b5a", "function_id": target.FunctionID, "namespace": target.Namespace,
			"activation": map[string]any{"observer_id": observers[source]}, "data": map[string]any{"ignored": true}})
		send <- out
	}
	before, listed := fetches.Load(), lists
	activate("configuration::on-updated")
	read("the settings read again after a configuration event", func() bool { return fetches.Load() > before && lists > listed })
	listed, fetched := lists, fetches.Load()
	activate("engine::on-functions-changed")
	read("a functions list after a functions event", func() bool { return lists > listed })
	if fetches.Load() != fetched {
		t.Fatal("a functions event read the settings")
	}
	added, _ := json.Marshal(map[string]any{"type": "observeradded", "function_id": StatusChanged, "namespace": "project", "revision": 1,
		"observer": map[string]any{"id": "page-1", "target": "iii::mesh-ui::events::console-1", "capture": []string{"input", "output", "caller"}, "on": "success"}})
	send <- added
	read("the current status for a new observer", func() bool { return registered["delivered"] != nil })
	cancel()
	<-done
}

func mustJSON(t *testing.T, v any) []byte {
	t.Helper()
	b, err := json.Marshal(v)
	if err != nil {
		t.Fatal(err)
	}
	return b
}
