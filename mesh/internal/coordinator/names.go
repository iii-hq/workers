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

package coordinator

import (
	"fmt"
	"strings"
)

func isReserved(s string) bool {
	switch s {
	case "mesh", "hub", "engine", "iii", "configuration", "harness", "shell", "coder":
		return true
	}
	return false
}

func SanitizeName(host string) string {
	host, _, _ = strings.Cut(strings.ToLower(host), ".")
	var b []byte
	for _, r := range host {
		switch {
		case r >= 'a' && r <= 'z' || r >= '0' && r <= '9':
			b = append(b, byte(r))
		case len(b) > 0 && b[len(b)-1] != '-':
			b = append(b, '-')
		}
	}
	s := strings.Trim(string(b), "-")
	if s == "" {
		return "node"
	}
	if s[0] < 'a' || s[0] > 'z' {
		s = "n-" + s
	}
	if len(s) > 31 {
		s = s[:31]
	}
	return strings.TrimRight(s, "-")
}

func AllocateName(host string, taken func(string) bool) string {
	base := SanitizeName(host)
	name := base
	for i := 2; isReserved(name) || taken != nil && taken(name); i++ {
		suffix := fmt.Sprintf("-%d", i)
		name = strings.TrimRight(base[:min(len(base), 31-len(suffix))], "-") + suffix
	}
	return name
}
