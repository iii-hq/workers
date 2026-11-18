package join

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"net/netip"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"tailscale.com/types/key"
)

func privateKey(t *testing.T) ed25519.PrivateKey {
	t.Helper()
	_, k, err := ed25519.GenerateKey(nil)
	if err != nil {
		t.Fatal(err)
	}
	return k
}

func TestParseCode(t *testing.T) {
	for _, tt := range []struct{ input, want string }{
		{"abcd-efgh", "ABCDEFGH"}, {"O1IL-0000", "01110000"}, {"ABCDEFGH", "ABCDEFGH"},
	} {
		got, err := ParseCode(tt.input)
		if err != nil || got != tt.want {
			t.Fatalf("ParseCode(%q) = %q, %v; want %q", tt.input, got, err, tt.want)
		}
	}
	for _, s := range []string{"ABC", "ABCDEFGU", "ABCDEFGH0", " ABCDEFGH", ""} {
		if _, err := ParseCode(s); !errors.Is(err, ErrCode) {
			t.Errorf("ParseCode(%q) error = %v; want ErrCode", s, err)
		}
	}
}

func TestNewCode(t *testing.T) {
	for range 100 {
		c := NewCode()
		if len(c) != 9 || c[4] != '-' {
			t.Fatalf("bad display format %q", c)
		}
		if _, err := ParseCode(c); err != nil {
			t.Fatal(err)
		}
	}
}

func TestSecretNormalizesCode(t *testing.T) {
	k := privateKey(t)
	c := NewCode()
	normalized, err := ParseCode(c)
	if err != nil {
		t.Fatal(err)
	}
	if Secret(k, c) != Secret(k, strings.ToLower(normalized)) {
		t.Fatal("equivalent codes derive different secrets")
	}
	if Secret(k, c) == Secret(privateKey(t), c) {
		t.Fatal("secret is not bound to identity")
	}
	if Secret(k, c) == Secret(k, NewCode()) {
		t.Fatal("code rotation did not change secret")
	}
}

func TestLimitRefusesAfterTen(t *testing.T) {
	now := time.Now()
	l := NewLimit(10, 10*time.Minute, func() time.Time { return now })
	for range 10 {
		l.Refused("a")
	}
	if l.Allow("a") == nil {
		t.Fatal("eleventh attempt allowed")
	}
	if err := l.Allow("b"); err != nil {
		t.Fatal(err)
	}
	now = now.Add(10 * time.Minute)
	if err := l.Allow("a"); err != nil {
		t.Fatal(err)
	}
	for range 10 {
		if err := l.Allow("a"); err != nil {
			t.Fatal("Allow must not count refusals:", err)
		}
	}
}

func TestClientAddrTrustsOnlyListedProxies(t *testing.T) {
	loopback := []netip.Prefix{netip.MustParsePrefix("127.0.0.1/32")}
	for _, tt := range []struct {
		name, peer, cf, xff, want string
		trusted                   []netip.Prefix
	}{
		{"untrusted CF", "203.0.113.9:1", "198.51.100.4", "", "203.0.113.9", nil},
		{"trusted CF", "127.0.0.1:1", "198.51.100.4", "", "198.51.100.4", loopback},
		{"untrusted XFF", "203.0.113.9:1", "", "198.51.100.4", "203.0.113.9", loopback},
		{"trusted XFF", "127.0.0.1:1", "", "198.51.100.4", "198.51.100.4", loopback},
		{"forged leftmost", "127.0.0.1:1", "", "192.0.2.1, 198.51.100.4", "198.51.100.4", loopback},
		{"trusted chain", "127.0.0.1:1", "", "198.51.100.4, 127.0.0.1", "198.51.100.4", loopback},
		{"invalid headers", "127.0.0.1:1", "bad", "bad", "127.0.0.1", loopback},
		{"IPv6 peer", "[2001:db8::1]:1", "198.51.100.4", "", "2001:db8::", nil},
		{"IPv6 same /64", "[2001:db8::ffff:1]:1", "", "", "2001:db8::", nil},
		{"IPv6 other /64", "[2001:db8:0:1::1]:1", "", "", "2001:db8:0:1::", nil},
		{"trusted CF IPv6", "127.0.0.1:1", "2001:db8::2", "", "2001:db8::", loopback},
		{"IPv4-mapped peer", "[::ffff:203.0.113.9]:1", "", "", "203.0.113.9", nil},
	} {
		t.Run(tt.name, func(t *testing.T) {
			r := httptest.NewRequest(http.MethodPost, "/join", nil)
			r.RemoteAddr = tt.peer
			r.Header.Set("CF-Connecting-IP", tt.cf)
			r.Header.Set("X-Forwarded-For", tt.xff)
			if got := ClientAddr(r, tt.trusted); got != tt.want {
				t.Fatalf("got %q; want %q", got, tt.want)
			}
		})
	}
}

func joinServer(t *testing.T, k ed25519.PrivateKey, c string, l *Limit, prefix string, id http.Handler) *httptest.Server {
	t.Helper()
	mux := http.NewServeMux()
	mux.Handle(prefix+"/join", Handler(k, func() string { return c }, l, nil))
	if id == nil {
		id = IdentityHandler(k, key.NewMachine().Public())
	}
	mux.Handle(prefix+"/id", id)
	s := httptest.NewServer(mux)
	t.Cleanup(s.Close)
	return s
}

func TestRedeemRoundTrip(t *testing.T) {
	k, c := privateKey(t), NewCode()
	l := NewLimit(10, 10*time.Minute, time.Now)
	s := joinServer(t, k, c, l, "", nil)
	secret, identity, err := Redeem(context.Background(), s.URL, strings.ToLower(c))
	if err != nil {
		t.Fatal(err)
	}
	if secret != Secret(k, c) || !bytes.Equal(identity, k.Public().(ed25519.PublicKey)) {
		t.Fatal("incorrect secret or identity")
	}
	if _, _, err := Redeem(context.Background(), s.URL, NewCode()); !errors.Is(err, ErrCode) {
		t.Fatalf("wrong code error = %v", err)
	}
	// One successful exchange and one failed exchange both consume a try.
	for range 8 {
		l.Refused("127.0.0.1")
	}
	if l.Allow("127.0.0.1") == nil {
		t.Fatal("exchange refusals were not counted")
	}
}

func TestRedeemPreservesURLPrefix(t *testing.T) {
	for _, suffix := range []string{"", "/"} {
		t.Run("trailing="+suffix, func(t *testing.T) {
			k, c := privateKey(t), NewCode()
			s := joinServer(t, k, c, NewLimit(10, 10*time.Minute, time.Now), "/mesh/public", nil)
			secret, _, err := Redeem(context.Background(), s.URL+"/mesh/public"+suffix, c)
			if err != nil || secret != Secret(k, c) {
				t.Fatalf("Redeem = %q, %v", secret, err)
			}
		})
	}
}

func TestRedeemAgainstImpostorFails(t *testing.T) {
	legitimate, impostor := privateKey(t), privateKey(t)
	c := NewCode()
	s := joinServer(t, impostor, c, NewLimit(10, 10*time.Minute, time.Now), "", IdentityHandler(legitimate, key.NewMachine().Public()))
	secret, identity, err := Redeem(context.Background(), s.URL, c)
	if err == nil || secret != "" || identity != nil {
		t.Fatalf("impostor accepted: %q, %x, %v", secret, identity, err)
	}
}

func TestIdentityVerifiesPinnedKey(t *testing.T) {
	k := privateKey(t)
	control := key.NewMachine().Public()
	s := httptest.NewServer(IdentityHandler(k, control))
	defer s.Close()
	doc, err := FetchIdentity(context.Background(), s.URL)
	if err != nil {
		t.Fatal(err)
	}
	if doc.Control != control {
		t.Fatal("wrong control key")
	}
	if err := doc.Verify(k.Public().(ed25519.PublicKey)); err != nil {
		t.Fatal(err)
	}
	if err := doc.Verify(privateKey(t).Public().(ed25519.PublicKey)); err == nil {
		t.Fatal("accepted identity different from pinned key")
	}
}

func TestRedeemRejectsInvalidIdentityBeforeExchange(t *testing.T) {
	k, c := privateKey(t), NewCode()
	good := IdentityHandler(k, key.NewMachine().Public())
	rr := httptest.NewRecorder()
	good.ServeHTTP(rr, httptest.NewRequest(http.MethodGet, "/id", nil))
	var original Identity
	if err := json.Unmarshal(rr.Body.Bytes(), &original); err != nil {
		t.Fatal(err)
	}
	for _, tt := range []struct {
		name   string
		mutate func(*Identity)
	}{
		{"signature", func(d *Identity) { d.Signature = nil }},
		{"control", func(d *Identity) { d.Control = key.NewMachine().Public() }},
		{"identity", func(d *Identity) { d.Identity = privateKey(t).Public().(ed25519.PublicKey) }},
		{"short identity", func(d *Identity) { d.Identity = []byte{1} }},
		{"zero control", func(d *Identity) {
			d.Control = key.MachinePublic{}
			d.Signature = ed25519.Sign(k, []byte("iii mesh control "+d.Control.String()))
		}},
	} {
		t.Run(tt.name, func(t *testing.T) {
			doc := original
			tt.mutate(&doc)
			var calls atomic.Int32
			s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if r.URL.Path == "/join" {
					calls.Add(1)
					t.Error("exchange attempted with invalid identity")
				}
				_ = json.NewEncoder(w).Encode(doc)
			}))
			defer s.Close()
			secret, identity, err := Redeem(context.Background(), s.URL, c)
			if err == nil || secret != "" || identity != nil || calls.Load() != 0 {
				t.Fatalf("invalid identity accepted: %q, %x, %v", secret, identity, err)
			}
		})
	}
}

func TestHandlerLimitsConcurrentSuccessfulExchanges(t *testing.T) {
	k, c := privateKey(t), NewCode()
	s := joinServer(t, k, c, NewLimit(10, 10*time.Minute, time.Now), "", nil)
	var successes atomic.Int32
	var wg sync.WaitGroup
	start := make(chan struct{})
	for range 30 {
		wg.Go(func() {
			<-start
			secret, _, err := Redeem(context.Background(), s.URL, c)
			if err == nil {
				if secret != Secret(k, c) {
					t.Error("wrong secret")
				}
				successes.Add(1)
			} else if !strings.Contains(err.Error(), "429") {
				t.Errorf("unexpected error: %v", err)
			}
		})
	}
	close(start)
	wg.Wait()
	if got := successes.Load(); got != 10 {
		t.Fatalf("successful exchanges = %d; want exactly 10", got)
	}
}

func TestHandlerRejectsInvalidRequests(t *testing.T) {
	k, c := privateKey(t), NewCode()
	for _, tt := range []struct {
		name, method, body string
		status             int
	}{
		{"method", http.MethodGet, "", http.StatusMethodNotAllowed},
		{"malformed JSON", http.MethodPost, "{", http.StatusBadRequest},
		{"invalid CPace", http.MethodPost, "{}", http.StatusBadRequest},
		{"oversized", http.MethodPost, strings.Repeat(" ", 1025), http.StatusBadRequest},
	} {
		t.Run(tt.name, func(t *testing.T) {
			h := Handler(k, func() string { return c }, NewLimit(10, 10*time.Minute, time.Now), nil)
			rr := httptest.NewRecorder()
			h.ServeHTTP(rr, httptest.NewRequest(tt.method, "/join", strings.NewReader(tt.body)))
			if rr.Code != tt.status {
				t.Fatalf("status = %d; want %d", rr.Code, tt.status)
			}
		})
	}
}
