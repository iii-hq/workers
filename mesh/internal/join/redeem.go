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
	"bytes"
	"context"
	"crypto/ed25519"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"time"

	"filippo.io/cpace"
	"tailscale.com/types/key"
)

type Identity struct {
	Identity  ed25519.PublicKey `json:"identity"`
	Control   key.MachinePublic `json:"control"`
	Signature []byte            `json:"signature"`
}

func controlMessage(control key.MachinePublic) string {
	return "iii mesh control " + control.String()
}

// Verify checks the signed control key and, when supplied, the enrolled
// identity. With no pin, only successful CPace authenticates this identity.
func (d Identity) Verify(pinned ed25519.PublicKey) error {
	if len(d.Identity) != ed25519.PublicKeySize || d.Control.IsZero() {
		return errors.New("invalid hub identity or control key")
	}
	if len(pinned) != 0 && !bytes.Equal(pinned, d.Identity) {
		return errors.New("hub identity differs from pinned identity")
	}
	if !ed25519.Verify(d.Identity, []byte(controlMessage(d.Control)), d.Signature) {
		return errors.New("invalid hub control key signature")
	}
	return nil
}

func endpoint(publicURL, path string) (string, error) {
	u, err := url.Parse(publicURL)
	if err != nil {
		return "", err
	}
	if (u.Scheme != "http" && u.Scheme != "https") || u.Host == "" || u.User != nil || u.RawQuery != "" || u.Fragment != "" {
		return "", errors.New("invalid hub public URL")
	}
	return u.JoinPath(path).String(), nil
}

func response(req *http.Request, value any) error {
	resp, err := (&http.Client{Timeout: 10 * time.Second}).Do(req)
	if err != nil {
		return err
	}
	defer func() { _ = resp.Body.Close() }()
	if resp.StatusCode != http.StatusOK {
		b, _ := io.ReadAll(io.LimitReader(resp.Body, maxJoin))
		return fmt.Errorf("%s: %s: %s", req.URL, resp.Status, bytes.TrimSpace(b))
	}
	return decode(resp.Body, value)
}

// FetchIdentity verifies the control key's signature. Call Verify with the
// enrolled identity on subsequent reads before trusting the returned control.
func FetchIdentity(ctx context.Context, publicURL string) (Identity, error) {
	idURL, err := endpoint(publicURL, "id")
	if err != nil {
		return Identity{}, err
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, idURL, nil)
	if err != nil {
		return Identity{}, err
	}
	var d Identity
	if err := response(req, &d); err != nil {
		return Identity{}, err
	}
	if err := d.Verify(nil); err != nil {
		return Identity{}, err
	}
	return d, nil
}

func Redeem(ctx context.Context, publicURL, code string) (secret string, identity ed25519.PublicKey, err error) {
	c, err := ParseCode(code)
	if err != nil {
		return "", nil, err
	}
	d, err := FetchIdentity(ctx, publicURL)
	if err != nil {
		return "", nil, err
	}
	a, s, err := cpace.Start(c, pake(d.Identity))
	if err != nil {
		return "", nil, err
	}
	body, err := json.Marshal(joinMsg{A: a})
	if err != nil {
		return "", nil, err
	}
	joinURL, err := endpoint(publicURL, "join")
	if err != nil {
		return "", nil, err
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, joinURL, bytes.NewReader(body))
	if err != nil {
		return "", nil, err
	}
	req.Header.Set("Content-Type", "application/json")
	var m joinMsg
	if err := response(req, &m); err != nil {
		return "", nil, err
	}
	isk, err := s.Finish(m.B)
	if err != nil {
		return "", nil, err
	}
	g, err := aead(isk)
	if err != nil {
		return "", nil, err
	}
	opened, err := g.Open(nil, nil, m.Box, nil)
	if err != nil {
		return "", nil, ErrCode
	}
	return string(opened), d.Identity, nil
}
