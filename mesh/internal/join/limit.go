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
	"fmt"
	"net"
	"net/http"
	"net/netip"
	"strings"
	"sync"
	"time"
)

// Limit shares one window across addresses, bounding the lifetime of the
// refusal table even when clients never return.
type Limit struct {
	mu     sync.Mutex
	m      map[string]int
	start  time.Time
	max    int
	window time.Duration
	now    func() time.Time
}

func NewLimit(max int, window time.Duration, now func() time.Time) *Limit {
	return &Limit{m: make(map[string]int), start: now(), max: max, window: window, now: now}
}

func (l *Limit) reset(now time.Time) {
	if now.Sub(l.start) >= l.window {
		l.start, l.m = now, make(map[string]int)
	}
}

func (l *Limit) allow(addr string, now time.Time) error {
	if l.m[addr] >= l.max {
		remaining := l.window - now.Sub(l.start)
		return fmt.Errorf("too many refused credentials from %s: try again in %s", addr, (remaining + time.Second - 1).Truncate(time.Second))
	}
	return nil
}

func (l *Limit) Allow(addr string) error {
	l.mu.Lock()
	defer l.mu.Unlock()
	now := l.now()
	l.reset(now)
	return l.allow(addr, now)
}

// take keeps admission and counting atomic so concurrent exchanges cannot
// all pass the check before any of them counts its attempt.
func (l *Limit) take(addr string) error {
	l.mu.Lock()
	defer l.mu.Unlock()
	now := l.now()
	l.reset(now)
	if err := l.allow(addr, now); err != nil {
		return err
	}
	l.m[addr]++
	return nil
}

func (l *Limit) Refused(addr string) {
	l.mu.Lock()
	defer l.mu.Unlock()
	l.reset(l.now())
	l.m[addr]++
}

func ClientAddr(r *http.Request, trusted []netip.Prefix) string {
	host, _, err := net.SplitHostPort(r.RemoteAddr)
	if err != nil {
		host = r.RemoteAddr
	}
	peer, err := netip.ParseAddr(host)
	if err != nil {
		return host
	}
	peer = peer.Unmap()
	isTrusted := func(addr netip.Addr) bool {
		for _, prefix := range trusted {
			if prefix.Contains(addr) {
				return true
			}
		}
		return false
	}
	if !isTrusted(peer) {
		return peer.String()
	}
	if addr, err := netip.ParseAddr(strings.TrimSpace(r.Header.Get("CF-Connecting-IP"))); err == nil {
		return addr.Unmap().String()
	}
	// Walk from the trusted peer toward the client; entries before the first
	// untrusted hop may have been supplied by that hop itself.
	forwarded := strings.Split(r.Header.Get("X-Forwarded-For"), ",")
	for i := len(forwarded) - 1; i >= 0 && isTrusted(peer); i-- {
		addr, err := netip.ParseAddr(strings.TrimSpace(forwarded[i]))
		if err != nil {
			break
		}
		peer = addr.Unmap()
	}
	return peer.String()
}
