package meshnet_test

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/iii-hq/workers/mesh/internal/hub"
	"github.com/iii-hq/workers/mesh/internal/join"
	"github.com/iii-hq/workers/mesh/internal/meshnet"
)

func TestHubTokenRealMeshEnrollment(t *testing.T) {
	t.Setenv("TS_NO_LOGS_NO_SUPPORT", "true")
	ctx, cancel := context.WithTimeout(t.Context(), 40*time.Second)
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
	issued, err := h.JoinToken(ctx, join.TokenOptions{Ephemeral: true, Reusable: true})
	if err != nil {
		t.Fatal(err)
	}
	env, err := join.DecodeToken(issued.Token)
	if err != nil {
		t.Fatal(err)
	}
	identity, err := join.FetchIdentity(ctx, env.PublicURL)
	if err != nil {
		t.Fatal(err)
	}
	if err = identity.Verify(env.Identity); err != nil {
		t.Fatal(err)
	}
	start := func(dir string) *meshnet.Mesh {
		t.Helper()
		shim, err := meshnet.NewShim(env.PublicURL, identity.Control)
		if err != nil {
			t.Fatal(err)
		}
		hosts := filepath.Join(t.TempDir(), "hosts")
		if err = os.WriteFile(hosts, []byte("127.0.0.1 localhost\n"), 0600); err != nil {
			t.Fatal(err)
		}
		m, err := meshnet.StartForTest(dir, shim, env.Secret, "runner", 0, nil, nil, hosts)
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() { m.Close() })
		return m
	}
	wait := func(label string, f func() bool) {
		t.Helper()
		deadline := time.After(10 * time.Second)
		for !f() {
			select {
			case <-deadline:
				t.Fatal(label)
			case <-time.After(10 * time.Millisecond):
			}
		}
	}
	dir := t.TempDir()
	m := start(dir)
	wait("token did not receive authenticated map", func() bool { return m.Self().IsValid() })
	address := m.Self()
	if p, ok := m.Peer("hub"); !ok || !p.Is6() {
		t.Fatal("missing hub peer")
	}
	if len(h.Coordinator.Nodes()) != 2 || !h.Coordinator.Nodes()[1].Ephemeral {
		t.Fatal("hub lost ephemeral grant")
	}
	code := h.Coordinator.Code()
	if _, err = h.RevokeToken(ctx, hub.RevokeTokenRequest{TokenID: issued.TokenID}); err != nil {
		t.Fatal(err)
	}
	if h.Coordinator.Code() != code {
		t.Fatal("revoke changed device code")
	}
	if err = m.Close(); err != nil {
		t.Fatal(err)
	}
	m = start(dir)
	wait("accepted node did not reconnect after revoke", func() bool { return m.Self().IsValid() })
	if m.Self() != address {
		t.Fatal("reconnect changed address")
	}
	removed := make(chan string, 1)
	h.SetHooks(hub.Hooks{RevokeNode: func(name string) { removed <- name }})
	if _, err = h.RemoveNode(ctx, hub.NodeRequest{Name: "runner"}); err != nil {
		t.Fatal(err)
	}
	select {
	case <-m.Removed():
	case <-ctx.Done():
		t.Fatal("production Mesh missed removal")
	}
	select {
	case name := <-removed:
		if name != "runner" {
			t.Fatal(name)
		}
	case <-ctx.Done():
		t.Fatal("hub revocation hook missing")
	}
	replacement, err := h.JoinToken(ctx, join.TokenOptions{Ephemeral: true, Reusable: true})
	if err != nil {
		t.Fatal(err)
	}
	env, err = join.DecodeToken(replacement.Token)
	if err != nil {
		t.Fatal(err)
	}
	current := start(t.TempDir())
	wait("new token did not enroll replacement node", func() bool { return current.Self().IsValid() })
	if _, err = h.RemoveNode(ctx, hub.NodeRequest{Name: "runner"}); err != nil {
		t.Fatal(err)
	}
	select {
	case <-current.Removed():
	case <-ctx.Done():
		t.Fatal("replacement missed removal")
	}
	fresh := start(t.TempDir())
	refusedCtx, stop := context.WithTimeout(ctx, 10*time.Second)
	defer stop()
	if message := meshnet.RegistrationErrorForTest(refusedCtx, fresh); !strings.Contains(message, "join token") {
		t.Fatalf("missing authenticated token refusal: %s", message)
	}
	if fresh.Self().IsValid() || len(h.Coordinator.Nodes()) != 1 {
		t.Fatal("removed token enrolled fresh keys")
	}
	if _, _, err = join.Redeem(ctx, h.FrontURL(), h.Coordinator.Code()); err != nil {
		t.Fatal("device code disabled by token revoke", err)
	}
}
