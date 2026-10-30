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
	"crypto/ed25519"
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base32"
	"errors"
	"strings"
)

const crockford = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"

var ErrCode = errors.New("wrong device code")

func NewCode() string {
	b := make([]byte, 8)
	_, _ = rand.Read(b)
	for i := range b {
		b[i] = crockford[b[i]&31]
	}
	return string(b[:4]) + "-" + string(b[4:])
}

func normalizeCode(s string) string {
	return strings.NewReplacer("-", "", "O", "0", "I", "1", "L", "1").Replace(strings.ToUpper(s))
}

func ParseCode(s string) (string, error) {
	c := normalizeCode(s)
	if len(c) != 8 || strings.Trim(c, crockford) != "" {
		return "", ErrCode
	}
	return c, nil
}

// Secret derives from the normalized code, so rotation invalidates old join
// credentials without keeping a table of secrets.
func Secret(k ed25519.PrivateKey, code string) string {
	m := hmac.New(sha256.New, k.Seed())
	m.Write([]byte("join " + normalizeCode(code)))
	return base32.StdEncoding.WithPadding(base32.NoPadding).EncodeToString(m.Sum(nil))
}
