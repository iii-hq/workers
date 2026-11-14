package node

import (
	"crypto/ed25519"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"sync"
	"time"
)

var (
	ErrUnsigned = errors.New("unsigned node request")
	ErrBadSig   = errors.New("invalid node signature")
	ErrSkew     = errors.New("node request clock skew exceeds window")
	ErrReplay   = errors.New("replayed node request")
)

type Signed struct {
	Payload json.RawMessage `json:"payload"`
	TS      int64           `json:"ts"`
	Nonce   string          `json:"nonce"`
	Sig     string          `json:"sig"`
}

// The signed identity is the registered mesh-node::* ID, before proxy prefixing.
func Sign(k ed25519.PrivateKey, fn string, payload any, now time.Time) (Signed, error) {
	if len(k) != ed25519.PrivateKeySize {
		return Signed{}, ErrBadSig
	}
	raw, err := json.Marshal(payload)
	if err != nil {
		return Signed{}, err
	}
	raw, err = canonicalPayload(raw)
	if err != nil {
		return Signed{}, err
	}
	nonce := make([]byte, 32)
	if _, err = rand.Read(nonce); err != nil {
		return Signed{}, err
	}
	s := Signed{Payload: raw, TS: now.Unix(), Nonce: hex.EncodeToString(nonce)}
	message, err := signatureMessage(fn, s)
	if err != nil {
		return Signed{}, err
	}
	s.Sig = hex.EncodeToString(ed25519.Sign(k, message))
	return s, nil
}

// Ordinary JSON intermediaries decode numbers to float64 and reorder object
// keys. Normalize at both ends so they do not invalidate an authentic envelope.
func canonicalPayload(raw json.RawMessage) (json.RawMessage, error) {
	var value any
	if err := json.Unmarshal(raw, &value); err != nil {
		return nil, err
	}
	return json.Marshal(value)
}
func signatureMessage(fn string, s Signed) ([]byte, error) {
	payload, err := canonicalPayload(s.Payload)
	if err != nil {
		return nil, err
	}
	return json.Marshal(struct {
		Function string          `json:"function"`
		Payload  json.RawMessage `json:"payload"`
		TS       int64           `json:"ts"`
		Nonce    string          `json:"nonce"`
	}{fn, payload, s.TS, s.Nonce})
}

type Verifier struct {
	pub    ed25519.PublicKey
	window time.Duration
	mu     sync.Mutex
	nonces map[string]time.Time
	skew   time.Duration
}

func NewVerifier(pub ed25519.PublicKey, window time.Duration) *Verifier {
	return &Verifier{pub: append(ed25519.PublicKey(nil), pub...), window: window, nonces: map[string]time.Time{}}
}
func (v *Verifier) Verify(fn string, s Signed, now time.Time) (json.RawMessage, error) {
	if s.Sig == "" || s.Nonce == "" || len(s.Payload) == 0 {
		return nil, ErrUnsigned
	}
	sig, err := hex.DecodeString(s.Sig)
	nonce, nonceErr := hex.DecodeString(s.Nonce)
	msg, msgErr := signatureMessage(fn, s)
	if err != nil || nonceErr != nil || len(nonce) != 32 || msgErr != nil || len(v.pub) != ed25519.PublicKeySize || !ed25519.Verify(v.pub, msg, sig) {
		return nil, ErrBadSig
	}
	v.mu.Lock()
	defer v.mu.Unlock()
	stamp := time.Unix(s.TS, 0)
	skew := stamp.Sub(now)
	if skew > v.window || skew < -v.window {
		v.skew = skew
		return nil, ErrSkew
	}
	for nonce, until := range v.nonces {
		if now.After(until) {
			delete(v.nonces, nonce)
		}
	}
	if _, ok := v.nonces[s.Nonce]; ok {
		return nil, ErrReplay
	}
	until := now.Add(v.window)
	if validity := stamp.Add(v.window); validity.After(until) {
		until = validity
	}
	v.nonces[s.Nonce] = until
	return canonicalPayload(s.Payload)
}
func (v *Verifier) LastSkew() time.Duration { v.mu.Lock(); defer v.mu.Unlock(); return v.skew }
