package hub

import (
	"bufio"
	"encoding/hex"
	"io"
	"net"
	"net/http"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

// Shared with quick-tunnel's tests: what opentunnel 0.6.2 writes for the server
// name r0ute234abcd5678.abc234.opentunnel.xyz.
const (
	proxyV4    = "0d0a0d0a000d0a515549540a21110035cb007107000000000fa001bb020026723075746532333461626364353637382e6162633233342e6f70656e74756e6e656c2e78797a"
	proxyV6    = "0d0a0d0a000d0a515549540a2121004d20010db80000000000000000000000010000000000000000000000000000000001bb01bb020026723075746532333461626364353637382e6162633233342e6f70656e74756e6e656c2e78797a"
	proxyLocal = "0d0a0d0a000d0a515549540a20000029020026723075746532333461626364353637382e6162633233342e6f70656e74756e6e656c2e78797a"
	plainGet   = "GET /id HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"
)

func unhex(t *testing.T, s string) string {
	t.Helper()
	b, err := hex.DecodeString(s)
	if err != nil {
		t.Fatal(err)
	}
	return string(b)
}

// proxyServer serves r.RemoteAddr behind a proxyListener and counts handler runs.
func proxyServer(t *testing.T, trust bool) (addr string, calls *atomic.Int32) {
	t.Helper()
	l, err := net.Listen("tcp4", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	calls = new(atomic.Int32)
	s := &http.Server{ReadHeaderTimeout: 10 * time.Second, Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		io.WriteString(w, r.RemoteAddr)
	})}
	go s.Serve(proxyListener{l, func() bool { return trust }})
	t.Cleanup(func() { s.Close() })
	return l.Addr().String(), calls
}

// exchange writes raw bytes, closes its write side when asked, and returns the
// status line and body ("" when the server closed without a response).
func exchange(t *testing.T, addr, raw string, closeWrite bool) (status, body string) {
	t.Helper()
	c, err := net.Dial("tcp", addr)
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	c.SetDeadline(time.Now().Add(10 * time.Second))
	if _, err := io.WriteString(c, raw); err != nil {
		t.Fatal(err)
	}
	if closeWrite {
		c.(*net.TCPConn).CloseWrite()
	}
	res, err := http.ReadResponse(bufio.NewReader(c), nil)
	if err != nil {
		return "", ""
	}
	defer res.Body.Close()
	b, _ := io.ReadAll(res.Body)
	return res.Status, string(b)
}

func TestProxyHeaderGivesVisitorAddress(t *testing.T) {
	addr, calls := proxyServer(t, true)
	for _, tc := range []struct{ header, want string }{
		{proxyV4, "203.0.113.7:4000"},
		{proxyV6, "[2001:db8::1]:443"},
	} {
		if status, body := exchange(t, addr, unhex(t, tc.header)+plainGet, false); status != "200 OK" || body != tc.want {
			t.Fatalf("got %q %q, want %q", status, body, tc.want)
		}
	}
	if calls.Load() != 2 {
		t.Fatalf("handler ran %d times", calls.Load())
	}
}

func TestProxyHeaderOptionalOnLoopback(t *testing.T) {
	addr, _ := proxyServer(t, true)
	for name, raw := range map[string]string{"no header": plainGet, "LOCAL": unhex(t, proxyLocal) + plainGet} {
		if status, body := exchange(t, addr, raw, false); status != "200 OK" || !strings.HasPrefix(body, "127.0.0.1:") {
			t.Fatalf("%s: got %q %q", name, status, body)
		}
	}
}

// fakeListener hands out one connection from a peer that is not loopback.
type fakeListener struct{ net.Listener }
type remoteConn struct{ net.Conn }

func (fakeListener) Accept() (net.Conn, error) { return remoteConn{}, nil }
func (remoteConn) RemoteAddr() net.Addr {
	return &net.TCPAddr{IP: net.IPv4(192, 0, 2, 1), Port: 1}
}

func TestProxyHeaderIgnoredWhenUntrusted(t *testing.T) {
	addr, calls := proxyServer(t, false)
	if status, _ := exchange(t, addr, unhex(t, proxyV4)+plainGet, false); status != "400 Bad Request" || calls.Load() != 0 {
		t.Fatalf("untrusted header: %q, handler ran %d times", status, calls.Load())
	}
	c, err := proxyListener{fakeListener{}, func() bool { return true }}.Accept()
	if err != nil {
		t.Fatal(err)
	}
	if _, wrapped := c.(*proxyConn); wrapped || c != (remoteConn{}) {
		t.Fatalf("a peer that is not loopback was wrapped: %T", c)
	}
}

func TestProxyHeaderMalformedClosesConnection(t *testing.T) {
	addr, calls := proxyServer(t, true)
	sig := string(proxySignature)
	for name, raw := range map[string]string{
		"version 3":      sig + "\x31\x11\x00\x0c" + strings.Repeat("\x00", 12) + plainGet,
		"length 600":     sig + "\x21\x11\x02\x58" + strings.Repeat("\x00", 600) + plainGet,
		"truncated body": sig + "\x21\x11\x00\x0c\xcb\x00",
	} {
		if status, _ := exchange(t, addr, raw, name == "truncated body"); status == "200 OK" {
			t.Fatalf("%s: served", name)
		}
	}
	if calls.Load() != 0 {
		t.Fatalf("handler ran %d times", calls.Load())
	}
}

func TestProxyHeaderIsReadOnce(t *testing.T) {
	addr, calls := proxyServer(t, true)
	forged := strings.Replace(proxyV4, "cb007107", "c6336464", 1) // 198.51.100.100
	status, body := exchange(t, addr, unhex(t, proxyV4)+unhex(t, forged)+plainGet, false)
	if status != "400 Bad Request" || calls.Load() != 0 || strings.Contains(body, "198.51.100.100") {
		t.Fatalf("second header parsed: %q %q, handler ran %d times", status, body, calls.Load())
	}
}
