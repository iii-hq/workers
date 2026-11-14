package join

import (
	"bytes"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"net/url"
	"strings"
	"time"
)

type Token struct {
	ID        string    `json:"id"`
	Hash      string    `json:"hash"`
	Expires   time.Time `json:"expires"`
	Reusable  bool      `json:"reusable"`
	Ephemeral bool      `json:"ephemeral"`
	Nodes     []string  `json:"nodes"`
	Revoked   bool      `json:"revoked,omitempty"`
}
type TokenOptions struct {
	Ephemeral     bool  `json:"ephemeral,omitempty"`
	Reusable      bool  `json:"reusable,omitempty"`
	ExpiresInSecs int64 `json:"expires_in_secs,omitempty"`
}
type TokenGrant struct {
	ID        string
	Ephemeral bool
}
type TokenEnvelope struct {
	PublicURL string            `json:"public_url"`
	Identity  ed25519.PublicKey `json:"identity"`
	Secret    string            `json:"secret"`
}

var ErrToken = errors.New("invalid, expired or revoked join token")

func IsTokenSecret(secret string) bool { return strings.HasPrefix(secret, "iiimjs_") }
func TokenHash(secret string) string {
	h := sha256.Sum256([]byte(secret))
	return hex.EncodeToString(h[:])
}

func NewToken(publicURL string, identity ed25519.PublicKey, opts TokenOptions) (Token, string, error) {
	if opts.Reusable && !opts.Ephemeral {
		return Token{}, "", errors.New("reusable requires ephemeral")
	}
	if opts.ExpiresInSecs < 0 || opts.ExpiresInSecs > int64((1<<63-1)/time.Second) {
		return Token{}, "", errors.New("invalid token expiry")
	}
	var secret [32]byte
	if _, err := rand.Read(secret[:]); err != nil {
		return Token{}, "", err
	}
	envelope := TokenEnvelope{publicURL, identity, "iiimjs_" + base64.RawURLEncoding.EncodeToString(secret[:])}
	b, err := json.Marshal(envelope)
	if err != nil {
		return Token{}, "", err
	}
	raw := "iiimj_" + base64.RawURLEncoding.EncodeToString(b)
	if _, err := DecodeToken(raw); err != nil {
		return Token{}, "", err
	}
	return Token{ID: rand.Text(), Hash: TokenHash(envelope.Secret), Reusable: opts.Reusable, Ephemeral: opts.Ephemeral, Nodes: []string{}}, raw, nil
}

func DecodeToken(raw string) (TokenEnvelope, error) {
	var e TokenEnvelope
	if len(raw) > 8192 {
		return e, ErrToken
	}
	encoded, ok := strings.CutPrefix(raw, "iiimj_")
	if !ok {
		return e, ErrToken
	}
	b, err := base64.RawURLEncoding.Strict().DecodeString(encoded)
	if err != nil {
		return e, ErrToken
	}
	d := json.NewDecoder(bytes.NewReader(b))
	d.DisallowUnknownFields()
	if err = d.Decode(&e); err != nil {
		return TokenEnvelope{}, ErrToken
	}
	if d.Decode(new(any)) != io.EOF {
		return TokenEnvelope{}, ErrToken
	}
	u, err := url.Parse(e.PublicURL)
	secret, prefix := strings.CutPrefix(e.Secret, "iiimjs_")
	decoded, secretErr := base64.RawURLEncoding.Strict().DecodeString(secret)
	if err != nil || (u.Scheme != "http" && u.Scheme != "https") || u.Hostname() == "" || u.User != nil || u.RawQuery != "" || u.ForceQuery || strings.Contains(e.PublicURL, "#") || len(e.Identity) != ed25519.PublicKeySize || !prefix || secretErr != nil || len(decoded) != 32 {
		return TokenEnvelope{}, ErrToken
	}
	return e, nil
}
