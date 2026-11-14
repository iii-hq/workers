package node

import (
	"crypto/ed25519"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"testing"
	"time"
)

func TestVerifyRefuses(t *testing.T) {
	pub, private, _ := ed25519.GenerateKey(rand.Reader)
	now := time.Unix(1700000000, 0)
	for _, name := range []string{"unsigned", "signature", "old", "replay", "valid", "wrong function"} {
		t.Run(name, func(t *testing.T) {
			v := NewVerifier(pub, time.Minute)
			s, err := Sign(private, "mesh-node::status", map[string]any{"hello": "world"}, now)
			if err != nil {
				t.Fatal(err)
			}
			fn := "mesh-node::status"
			var want error
			switch name {
			case "unsigned":
				s = Signed{}
				want = ErrUnsigned
			case "signature":
				b, _ := hex.DecodeString(s.Sig)
				b[0] ^= 1
				s.Sig = hex.EncodeToString(b)
				want = ErrBadSig
			case "old":
				s, _ = Sign(private, fn, s.Payload, now.Add(-61*time.Second))
				want = ErrSkew
			case "replay":
				if _, err := v.Verify(fn, s, now); err != nil {
					t.Fatal(err)
				}
				want = ErrReplay
			case "wrong function":
				fn = "mesh-node::workers::apply"
				want = ErrBadSig
			}
			got, err := v.Verify(fn, s, now)
			if !errors.Is(err, want) {
				t.Fatalf("got %v want %v", err, want)
			}
			if want == nil && string(got) != `{"hello":"world"}` {
				t.Fatalf("payload %s", got)
			}
		})
	}
}

func TestSignatureSurvivesJSONTransport(t *testing.T) {
	pub, k, _ := ed25519.GenerateKey(rand.Reader)
	now := time.Now()
	s, err := Sign(k, "mesh-node::workers::apply", json.RawMessage(` { "workers": [{"version":"1.0", "name":"ide"}], "namespace":"proj", "n":1.0 } `), now)
	if err != nil {
		t.Fatal(err)
	}
	b, _ := json.Marshal(s)
	var wire any
	json.Unmarshal(b, &wire)
	b, _ = json.Marshal(wire)
	json.Unmarshal(b, &s)
	if _, err := NewVerifier(pub, time.Minute).Verify("mesh-node::workers::apply", s, now); err != nil {
		t.Fatal(err)
	}
}

func TestReplayRetainedForFutureTimestamp(t *testing.T) {
	pub, k, _ := ed25519.GenerateKey(rand.Reader)
	now := time.Unix(1700000000, 0)
	s, _ := Sign(k, "mesh-node::status", struct{}{}, now.Add(59*time.Second))
	v := NewVerifier(pub, time.Minute)
	if _, err := v.Verify("mesh-node::status", s, now); err != nil {
		t.Fatal(err)
	}
	if _, err := v.Verify("mesh-node::status", s, now.Add(61*time.Second)); !errors.Is(err, ErrReplay) {
		t.Fatalf("future replay accepted: %v", err)
	}
}

func TestUnauthenticatedRequestsDoNotPoisonVerifier(t *testing.T) {
	pub, k, _ := ed25519.GenerateKey(rand.Reader)
	now := time.Now()
	s, _ := Sign(k, "mesh-node::status", struct{}{}, now)
	v := NewVerifier(pub, time.Minute)
	bad := s
	bad.TS += 90
	if _, err := v.Verify("mesh-node::status", bad, now); !errors.Is(err, ErrBadSig) {
		t.Fatal(err)
	}
	if v.LastSkew() != 0 {
		t.Fatal("unauthenticated skew recorded")
	}
	if _, err := v.Verify("mesh-node::status", s, now); err != nil {
		t.Fatal(err)
	}
}
