package cli

import (
	"context"
	"os"
	"testing"
	"time"

	"github.com/iii-hq/workers/mesh/internal/hub"
	"github.com/iii-hq/workers/mesh/internal/join"
	"github.com/iii-hq/workers/mesh/internal/meshnet"
	"tailscale.com/tsnet"
)

func TestHubTokenCLIEnrollment(t *testing.T) {
	t.Setenv("TS_NO_LOGS_NO_SUPPORT", "true")
	ctx, cancel := context.WithTimeout(t.Context(), 25*time.Second)
	defer cancel()
	cfg := hub.DefaultConfig()
	cfg.Listen = "127.0.0.1:0"
	h, err := hub.Start(ctx, hub.Options{DataDir: t.TempDir(), Version: "0.1.0"}, cfg, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer h.Close()
	cfg.PublicURL = h.FrontURL()
	if err = h.ApplyConfig(ctx, cfg); err != nil {
		t.Fatal(err)
	}
	issued, err := h.JoinToken(ctx, join.TokenOptions{})
	if err != nil {
		t.Fatal(err)
	}
	a, _, _ := testApp(t)
	os.RemoveAll(a.paths.State)
	a.operator = func() (string, error) { return "alice", nil }
	a.checkOperator = func(string) error { return nil }
	a.resolveOperator = func(string) (string, string, error) { return "DOMAIN\\alice", "saved-operator-id", nil }
	a.hostname = func() (string, error) { return "Laptop", nil }
	a.identity = join.FetchIdentity
	a.executable = func() (string, error) { return "/verified-download", nil }
	a.install = func(bin, operator string) error {
		saved, err := loadState(a.paths.State)
		if err != nil {
			return err
		}
		if bin != "/verified-download" || operator != "saved-operator-id" || saved.Operator != "DOMAIN\\alice" || saved.OperatorID != operator {
			t.Fatal("service ownership changed")
		}
		shim, err := meshnet.NewShim(saved.PublicURL, saved.Identity.Control)
		if err != nil {
			return err
		}
		t.Cleanup(func() { shim.Close() })
		network := &tsnet.Server{Dir: t.TempDir(), Hostname: saved.RequestedName, ControlURL: shim.URL(), AuthKey: saved.Secret, Port: uint16(saved.Port), Logf: func(string, ...any) {}, UserLogf: func(string, ...any) {}}
		t.Cleanup(func() { network.Close() })
		_, err = network.Up(ctx)
		return err
	}
	args, err := ParseJoin([]string{"--token", issued.Token, "--port=0"})
	if err != nil {
		t.Fatal(err)
	}
	if err = a.enroll(ctx, args); err != nil {
		t.Fatal(err)
	}
	nodes, err := h.ListNodes(ctx, struct{}{})
	if err != nil || len(nodes) != 1 || nodes[0].Name != "laptop" || nodes[0].Ephemeral {
		t.Fatal("CLI enrollment missing", err)
	}
	env, _ := join.DecodeToken(issued.Token)
	if _, err = h.Coordinator.RedeemToken(env.Secret); err == nil {
		t.Fatal("CLI registration did not consume single-use token")
	}
}
