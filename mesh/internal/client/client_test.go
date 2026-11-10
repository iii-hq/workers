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

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/coordinator"
	"github.com/iii-hq/workers/mesh/internal/elevate"
	"github.com/iii-hq/workers/mesh/internal/join"
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
	if _, err := os.Stat(filepath.Join(dir, "join.json")); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("join.json written before the hub accepted it: %v", err)
	}
	if err = savePrivate(dir, "join.json", st); err != nil { // as start does once registered
		t.Fatal(err)
	}
	for path, mode := range map[string]os.FileMode{dir: 0700, filepath.Join(dir, "join.json"): 0600} {
		if fi, err := os.Stat(path); err != nil || fi.Mode().Perm() != mode {
			t.Fatalf("%s: %v %v", path, fi.Mode(), err)
		}
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
	if _, err := elevate.Operator(); err != nil {
		t.Skip(err)
	}
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
