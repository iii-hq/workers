// MIT License
//
// Copyright (c) 2026 Alexander Yue
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

package join

import (
	"crypto/aes"
	"crypto/cipher"
	"crypto/ed25519"
	"crypto/hkdf"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/netip"

	"filippo.io/cpace"
	"tailscale.com/types/key"
)

// Identity binding prevents a different hub, even one knowing the code,
// from completing an exchange advertised as this hub.
func pake(identity ed25519.PublicKey) *cpace.ContextInfo {
	return cpace.NewContextInfo("worker", "harness", []byte("mainplane join "+base64.RawURLEncoding.EncodeToString(identity)))
}

func aead(isk []byte) (cipher.AEAD, error) {
	k, err := hkdf.Key(sha256.New, isk, nil, "mainplane join secret", 32)
	if err != nil {
		return nil, err
	}
	b, err := aes.NewCipher(k)
	if err != nil {
		return nil, err
	}
	return cipher.NewGCMWithRandomNonce(b)
}

type joinMsg struct {
	A   []byte `json:"a,omitempty"`
	B   []byte `json:"b,omitempty"`
	Box []byte `json:"box,omitempty"`
}

const maxJoin = 1 << 10

func decode(r io.Reader, value any) error {
	b, err := io.ReadAll(io.LimitReader(r, maxJoin+1))
	if err != nil {
		return err
	}
	if len(b) > maxJoin {
		return errors.New("join message too large")
	}
	return json.Unmarshal(b, value)
}

func Handler(k ed25519.PrivateKey, code func() string, l *Limit, trusted []netip.Prefix) http.Handler {
	identity := k.Public().(ed25519.PublicKey)
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPost {
			w.Header().Set("Allow", http.MethodPost)
			http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
			return
		}
		// CPace reveals no password verdict to the server, so every exchange
		// consumes a try, including exchanges with the correct code.
		if err := l.take(ClientAddr(r, trusted)); err != nil {
			http.Error(w, err.Error(), http.StatusTooManyRequests)
			return
		}
		var m joinMsg
		if err := decode(r.Body, &m); err != nil {
			http.Error(w, err.Error(), http.StatusBadRequest)
			return
		}
		c, err := ParseCode(code())
		if err != nil {
			http.Error(w, "invalid hub device code", http.StatusInternalServerError)
			return
		}
		b, isk, err := cpace.Exchange(c, pake(identity), m.A)
		if err != nil {
			http.Error(w, err.Error(), http.StatusBadRequest)
			return
		}
		g, err := aead(isk)
		if err != nil {
			http.Error(w, err.Error(), http.StatusInternalServerError)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(joinMsg{B: b, Box: g.Seal(nil, nil, []byte(Secret(k, c)), nil)})
	})
}

func IdentityHandler(k ed25519.PrivateKey, control key.MachinePublic) http.Handler {
	doc := Identity{
		Identity:  k.Public().(ed25519.PublicKey),
		Control:   control,
		Signature: ed25519.Sign(k, []byte(controlMessage(control))),
	}
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet {
			w.Header().Set("Allow", http.MethodGet)
			http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(doc)
	})
}
