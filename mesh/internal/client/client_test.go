package client

import (
	"context"
	"crypto/ed25519"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
	"time"

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
	for _, c := range []struct{ hubURL, value string }{{"", "ABCD-EFGH"}, {"http://127.0.0.1:1", "ABCD-EFGH"}, {"", token}} {
		t.Setenv("MESH_JOIN_TEST", c.value)
		st, _, err := r.join(context.Background(), Config{HubURL: c.hubURL, Join: "env://MESH_JOIN_TEST"})
		if s := r.status(); st != nil || err == nil || s.State != "waiting for join" || r.tried != "" {
			t.Fatalf("%+v: %v, status %+v, tried %q", c, err, s, r.tried)
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

// A build without a version would join, fail node.New and leave every tick.
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

// A node the hub removed must not rejoin on the next tick.
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
