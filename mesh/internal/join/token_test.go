package join

import (
	"crypto/ed25519"
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"strings"
	"testing"
)

func TestTokenEnvelopePinsExactHub(t *testing.T) {
	pub, _, _ := ed25519.GenerateKey(rand.Reader)
	_, raw, err := NewToken("https://hub.example/mesh", pub, TokenOptions{})
	if err != nil {
		t.Fatal(err)
	}
	got, err := DecodeToken(raw)
	if err != nil || got.PublicURL != "https://hub.example/mesh" || !pub.Equal(got.Identity) || !IsTokenSecret(got.Secret) {
		t.Fatal("invalid token envelope", err)
	}
	if !strings.HasPrefix(raw, "iiimj_") {
		t.Fatal("missing token prefix")
	}
	for _, change := range []func(*TokenEnvelope){
		func(e *TokenEnvelope) { e.PublicURL = "https://hub.example/?" },
		func(e *TokenEnvelope) { e.PublicURL = "https://user@hub.example/" },
		func(e *TokenEnvelope) { e.Identity = nil },
		func(e *TokenEnvelope) { e.Secret = rand.Text() },
	} {
		changed := got
		change(&changed)
		b, _ := json.Marshal(changed)
		if _, err := DecodeToken("iiimj_" + base64.RawURLEncoding.EncodeToString(b)); err == nil {
			t.Fatal("accepted malformed envelope")
		}
	}
	for _, raw := range []string{"", "iiimj_", "iiimj_!", raw + "=", raw + strings.Repeat("a", 8192)} {
		if _, err := DecodeToken(raw); err == nil {
			t.Fatal("accepted malformed token")
		}
	}
}

func TestReusableRequiresEphemeral(t *testing.T) {
	pub, _, _ := ed25519.GenerateKey(rand.Reader)
	if _, _, err := NewToken("https://hub.example", pub, TokenOptions{Reusable: true}); err == nil || err.Error() != "reusable requires ephemeral" {
		t.Fatal(err)
	}
}
