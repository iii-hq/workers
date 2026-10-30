package cli

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"github.com/iii-hq/workers/mesh/internal/join"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"tailscale.com/types/key"
	"testing"
	"time"
)

func TestJoinArgs(t *testing.T) {
	for _, args := range [][]string{{"https://hub.example", "ABCD-EFGH"}, {"--port", "41700", "https://hub.example", "ABCD-EFGH"}, {"https://hub.example", "ABCD-EFGH", "--port=0"}} {
		got, err := ParseJoin(args)
		if err != nil {
			t.Fatal(err)
		}
		want := 41643
		if len(args) == 4 {
			want = 41700
		}
		if len(args) == 3 {
			want = 0
		}
		if got.Port != want || got.PublicURL != "https://hub.example" || got.Code != "ABCD-EFGH" {
			t.Fatal(got)
		}
	}
	for _, args := range [][]string{nil, {"https://h"}, {"https://h", "bad"}, {"https://h", "ABCD-EFGH", "--port=65536"}, {"https://h", "ABCD-EFGH", "--port=-1"}, {"file:///tmp/x", "ABCD-EFGH"}, {"https://h", "ABCD-EFGH", "extra"}} {
		if _, err := ParseJoin(args); err == nil {
			t.Fatalf("accepted %v", args)
		}
	}
}

func TestJoinPersistsBeforeServiceStart(t *testing.T) {
	a, _, _ := testApp(t)
	os.RemoveAll(a.paths.State)
	s := testState(t)
	a.operator = func() (string, error) { return "alice", nil }
	a.checkOperator = func(string) error { return nil }
	a.hostname = func() (string, error) { return "Laptop", nil }
	a.redeem = func(context.Context, string, string) (string, ed25519.PublicKey, error) {
		return s.Secret, s.Identity.Identity, nil
	}
	a.identity = func(context.Context, string) (join.Identity, error) { return s.Identity, nil }
	a.executable = func() (string, error) { return "/tmp/downloaded-mesh", nil }
	a.install = func(bin, op string) error {
		saved, err := loadState(a.paths.State)
		if err != nil {
			t.Fatal(err)
		}
		if saved.Operator != "alice" || saved.RequestedName != "laptop" || saved.Port != 41700 || bin != "/tmp/downloaded-mesh" || op != "alice" {
			t.Fatalf("%+v %s %s", saved, bin, op)
		}
		return nil
	}
	if err := a.join(context.Background(), "https://hub.example", "ABCD-EFGH", 41700); err != nil {
		t.Fatal(err)
	}
	a.redeem = func(context.Context, string, string) (string, ed25519.PublicKey, error) {
		t.Fatal("redeemed on installed node")
		return "", nil, nil
	}
	if err := a.join(context.Background(), "https://hub.example", "ABCD-EFGH", 41700); err == nil {
		t.Fatal("overwrote installed state")
	}
}

func TestJoinRealRedeemAndIdentityPin(t *testing.T) {
	a, _, _ := testApp(t)
	os.RemoveAll(a.paths.State)
	_, private, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	control := key.NewMachine().Public()
	code := join.NewCode()
	mux := http.NewServeMux()
	mux.Handle("/id", join.IdentityHandler(private, control))
	mux.Handle("/join", join.Handler(private, func() string { return code }, join.NewLimit(10, 10*time.Minute, time.Now), nil))
	server := httptest.NewServer(mux)
	defer server.Close()
	a.operator = func() (string, error) { return "alice", nil }
	a.checkOperator = func(string) error { return nil }
	a.hostname = func() (string, error) { return "mesh", nil }
	a.redeem = join.Redeem
	a.identity = join.FetchIdentity
	a.executable = func() (string, error) { return "/download", nil }
	a.install = func(string, string) error { return nil }
	if err := a.join(context.Background(), server.URL, code, 41643); err != nil {
		t.Fatal(err)
	}
	saved, err := loadState(a.paths.State)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(saved.Identity.Identity, private.Public().(ed25519.PublicKey)) || saved.Identity.Control != control || saved.Secret != join.Secret(private, code) || saved.RequestedName != "node-mesh" {
		t.Fatal("enrollment pin/secret/name not preserved")
	}
}
func TestJoinRefusesChangedIdentityBeforeInstallation(t *testing.T) {
	a, _, _ := testApp(t)
	os.RemoveAll(a.paths.State)
	one, two := testState(t), testState(t)
	a.operator = func() (string, error) { return "alice", nil }
	a.checkOperator = func(string) error { return nil }
	a.hostname = func() (string, error) { return "laptop", nil }
	a.redeem = func(context.Context, string, string) (string, ed25519.PublicKey, error) {
		return one.Secret, one.Identity.Identity, nil
	}
	a.identity = func(context.Context, string) (join.Identity, error) { return two.Identity, nil }
	a.install = func(string, string) error { t.Fatal("installed unpinned identity"); return nil }
	if err := a.join(context.Background(), "https://hub.example", "ABCD-EFGH", 41643); err == nil {
		t.Fatal("accepted swapped identity")
	}
	if exists(filepath.Join(a.paths.State, "join.json")) {
		t.Fatal("persisted unpinned identity")
	}
}

func TestTokenJoinArgs(t *testing.T) {
	s := testState(t)
	_, raw, err := join.NewToken("https://hub.example/prefix", s.Identity.Identity, join.TokenOptions{})
	if err != nil {
		t.Fatal(err)
	}
	for _, args := range [][]string{{"--token", raw}, {"--port=0", "--token=" + raw}} {
		parsed, err := ParseJoin(args)
		if err != nil {
			t.Fatal(err)
		}
		if parsed.Token != raw || parsed.PublicURL != "https://hub.example/prefix" || parsed.Code != "" {
			t.Fatal("lost token enrollment arguments")
		}
	}
	for _, args := range [][]string{{"--token"}, {"--token", "invalid"}, {"--token", raw, "https://h", "ABCD-EFGH"}, {"--token", raw, "--token", raw}, {"--token", raw, "--port=1", "--port=2"}, {"--port=1", "--port=2", "https://h", "ABCD-EFGH"}} {
		if _, err := ParseJoin(args); err == nil {
			t.Fatal("accepted ambiguous/malformed join arguments")
		}
	}
}

func TestTokenJoinPinsIdentityAndOperator(t *testing.T) {
	a, _, _ := testApp(t)
	os.RemoveAll(a.paths.State)
	s := testState(t)
	_, raw, _ := join.NewToken("https://hub.example", s.Identity.Identity, join.TokenOptions{})
	a.operator = func() (string, error) { return "alice", nil }
	a.checkOperator = func(string) error { return nil }
	a.resolveOperator = func(string) (string, string, error) { return "DOMAIN\\alice", "canonical-operator-id", nil }
	a.hostname = func() (string, error) { return "Laptop", nil }
	a.identity = func(context.Context, string) (join.Identity, error) { return s.Identity, nil }
	a.redeem = func(context.Context, string, string) (string, ed25519.PublicKey, error) {
		t.Fatal("token invoked device-code exchange")
		return "", nil, nil
	}
	a.executable = func() (string, error) { return "/download", nil }
	a.install = func(bin, op string) error {
		saved, err := loadState(a.paths.State)
		if err != nil {
			t.Fatal(err)
		}
		if op != "canonical-operator-id" || saved.Operator != "DOMAIN\\alice" || saved.OperatorID != op || saved.RequestedName != "laptop" || saved.Port != 0 {
			t.Fatal("lost canonical ownership")
		}
		if saved.Secret == raw || !join.IsTokenSecret(saved.Secret) {
			t.Fatal("stored raw envelope or lost auth credential")
		}
		return nil
	}
	args, _ := ParseJoin([]string{"--token", raw, "--port=0"})
	if err := a.enroll(context.Background(), args); err != nil {
		t.Fatal(err)
	}
	os.RemoveAll(a.paths.State)
	other := testState(t)
	a.identity = func(context.Context, string) (join.Identity, error) { return other.Identity, nil }
	a.install = func(string, string) error { t.Fatal("installed unpinned hub"); return nil }
	if err := a.enroll(context.Background(), args); err == nil {
		t.Fatal("accepted swapped token identity")
	}
	if exists(filepath.Join(a.paths.State, "join.json")) {
		t.Fatal("saved unpinned enrollment")
	}
}
