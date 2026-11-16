package hub

import (
	"context"
	"testing"

	"github.com/iii-hq/workers/mesh/internal/coordinator"
	"github.com/iii-hq/workers/mesh/internal/join"
)

func TestTokenIssueAndRevoke(t *testing.T) {
	c, err := coordinator.New(t.TempDir(), nil)
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	h := &Hub{Coordinator: c, Version: "0.1.0", config: DefaultConfig()}
	if _, err := h.JoinToken(context.Background(), join.TokenOptions{}); err == nil {
		t.Fatal("issued without public URL")
	}
	h.config.PublicURL = "https://hub.example/prefix"
	before := c.Code()
	issued, err := h.JoinToken(context.Background(), join.TokenOptions{Ephemeral: true, Reusable: true})
	if err != nil {
		t.Fatal(err)
	}
	env, err := join.DecodeToken(issued.Token)
	if err != nil || env.PublicURL != h.config.PublicURL {
		t.Fatal("wrong enrollment URL", err)
	}
	if _, err := c.RedeemToken(env.Secret); err != nil {
		t.Fatal(err)
	}
	result, err := h.RevokeToken(context.Background(), RevokeTokenRequest{TokenID: issued.TokenID})
	if err != nil || !result.Revoked {
		t.Fatal(err)
	}
	if _, err := c.RedeemToken(env.Secret); err == nil {
		t.Fatal("revoked token accepted")
	}
	if c.Code() != before {
		t.Fatal("token changed device code")
	}
}
