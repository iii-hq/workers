package meshnet

import (
	"context"
	"crypto/x509"
	"errors"
	"fmt"
	"io"
	"log"
	"net"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/coder/websocket"
	"tailscale.com/control/controlhttp"
	"tailscale.com/control/controlhttp/controlhttpcommon"
	"tailscale.com/control/controlhttp/controlhttpserver"
	"tailscale.com/tailcfg"
	"tailscale.com/types/key"
)

func noiseEcho(ctx context.Context, shim *Shim, pinned key.MachinePublic) error {
	u, _ := url.Parse(shim.URL())
	d := controlhttp.Dialer{Hostname: u.Hostname(), HTTPPort: u.Port(), HTTPSPort: controlhttp.NoPort,
		MachineKey: key.NewMachine(), ControlKey: pinned, ProtocolVersion: uint16(tailcfg.CurrentCapabilityVersion)}
	c, err := d.Dial(ctx)
	if err != nil {
		return err
	}
	defer c.Close()
	deadline, _ := ctx.Deadline()
	c.SetDeadline(deadline)
	if _, err := c.Write([]byte("mesh")); err != nil {
		return err
	}
	var echo [4]byte
	if _, err := io.ReadFull(c, echo[:]); err != nil {
		return err
	}
	if string(echo[:]) != "mesh" {
		return fmt.Errorf("Noise echo = %q", echo)
	}
	return nil
}

func TestShimControlRecoversTransientUpgrade(t *testing.T) {
	for _, status := range []int{502, 503, 504} {
		t.Run(fmt.Sprint(status), func(t *testing.T) {
			var calls atomic.Int32
			failedClosed := make(chan struct{})
			machine := key.NewMachine()
			front := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if calls.Add(1) == 1 {
					w.WriteHeader(status)
					io.WriteString(w, strings.Repeat("x", 2048))
					w.(http.Flusher).Flush()
					<-r.Context().Done()
					close(failedClosed)
					return
				}
				c, err := controlhttpserver.AcceptHTTP(r.Context(), w, r, machine, nil)
				if err != nil {
					return
				}
				defer c.Close()
				io.Copy(c, c)
			}))
			defer front.Close()
			shim := newTestShim(t, front.URL, machine.Public())
			ctx, cancel := context.WithTimeout(t.Context(), time.Second)
			defer cancel()
			if err := noiseEcho(ctx, shim, machine.Public()); err != nil {
				t.Fatalf("first native dial failed despite healthy front door: %v (upstream attempts %d)", err, calls.Load())
			}
			if got := calls.Load(); got != 2 {
				t.Fatalf("upstream attempts = %d, want 2", got)
			}
			select {
			case <-failedClosed:
			case <-ctx.Done():
				t.Fatal("failed response body left upstream connection open")
			}
		})
	}
}

func TestShimControlRecoversListenerOutage(t *testing.T) {
	machine := key.NewMachine()
	var calls atomic.Int32
	down := make(chan struct{})
	front := httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {}))
	front.Config.Handler = http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if calls.Add(1) == 1 {
			front.Listener.Close()
			c, _, err := w.(http.Hijacker).Hijack()
			if err == nil {
				c.Close()
			}
			close(down)
			return
		}
		c, err := controlhttpserver.AcceptHTTP(r.Context(), w, r, machine, nil)
		if err != nil {
			return
		}
		defer c.Close()
		io.Copy(c, c)
	})
	front.Start()
	defer front.Close()
	shim := newTestShim(t, front.URL, machine.Public())
	ctx, cancel := context.WithTimeout(t.Context(), 2*time.Second)
	defer cancel()
	done := make(chan error, 1)
	go func() { done <- noiseEcho(ctx, shim, machine.Public()) }()
	select {
	case <-down:
	case <-ctx.Done():
		t.Fatal("first upstream attempt never arrived")
	}
	// Hold a real closed-listener interval across the initial retry backoff.
	<-time.After(100 * time.Millisecond)
	listener, err := net.Listen("tcp", front.Listener.Addr().String())
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()
	go front.Config.Serve(listener)
	if err := <-done; err != nil {
		t.Fatalf("first native dial did not recover EOF and listener outage: %v", err)
	}
	if got := calls.Load(); got != 2 {
		t.Fatalf("upstream requests = %d, want failed request and recovered Noise request", got)
	}
}

func controlUpgradeRequest(ctx context.Context, shim *Shim, path string) (*http.Response, error) {
	r, _ := http.NewRequestWithContext(ctx, http.MethodGet, shim.URL()+path, nil)
	r.Header.Set("Connection", "Upgrade")
	protocol := controlhttpcommon.UpgradeHeaderValue
	if path == "/derp" {
		protocol = "derp"
	}
	r.Header.Set("Upgrade", protocol)
	r.Header.Set(controlhttpcommon.HandshakeHeaderName, "aGVsbG8=")
	return http.DefaultClient.Do(r)
}

func TestShimControlPermanentRefusal(t *testing.T) {
	for _, status := range []int{400, 401, 403, 404, 429, 500} {
		t.Run(fmt.Sprint(status), func(t *testing.T) {
			var calls atomic.Int32
			front := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				calls.Add(1)
				http.Error(w, "refused", status)
			}))
			defer front.Close()
			shim := newTestShim(t, front.URL, key.NewMachine().Public())
			ctx, cancel := context.WithTimeout(t.Context(), time.Second)
			defer cancel()
			resp, err := controlUpgradeRequest(ctx, shim, "/ts2021")
			if err != nil {
				t.Fatal(err)
			}
			resp.Body.Close()
			if resp.StatusCode != 502 || calls.Load() != 1 {
				t.Fatalf("status = %d, upstream attempts = %d", resp.StatusCode, calls.Load())
			}
		})
	}
}

func TestShimControlProtocolMismatchClosesConnection(t *testing.T) {
	var calls atomic.Int32
	closed := make(chan struct{})
	front := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		c, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer c.CloseNow()
		c.Read(r.Context())
		close(closed)
	}))
	defer front.Close()
	shim := newTestShim(t, front.URL, key.NewMachine().Public())
	ctx, cancel := context.WithTimeout(t.Context(), time.Second)
	defer cancel()
	resp, err := controlUpgradeRequest(ctx, shim, "/ts2021")
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
	if resp.StatusCode != 502 || calls.Load() != 1 {
		t.Fatalf("status = %d, upstream attempts = %d", resp.StatusCode, calls.Load())
	}
	select {
	case <-closed:
	case <-ctx.Done():
		t.Fatal("mismatched WebSocket left open")
	}
}

func TestShimControlRetriesStopOnCancellation(t *testing.T) {
	for _, closeShim := range []bool{false, true} {
		for _, blockDial := range []bool{false, true} {
			t.Run(fmt.Sprintf("close=%t/inflight=%t", closeShim, blockDial), func(t *testing.T) {
				var calls atomic.Int32
				retrying := make(chan struct{})
				released := make(chan struct{})
				front := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
					if calls.Add(1) == 2 {
						close(retrying)
						if blockDial {
							<-r.Context().Done()
							close(released)
							return
						}
					}
					http.Error(w, "restarting", 503)
				}))
				defer front.Close()
				shim := newTestShim(t, front.URL, key.NewMachine().Public())
				ctx, cancel := context.WithCancel(t.Context())
				defer cancel()
				done := make(chan struct{})
				go func() {
					defer close(done)
					resp, _ := controlUpgradeRequest(ctx, shim, "/ts2021")
					if resp != nil {
						resp.Body.Close()
					}
				}()
				select {
				case <-retrying:
				case <-time.After(time.Second):
					t.Fatal("control upgrade did not retry")
				}
				stopped := make(chan struct{})
				go func() {
					defer close(stopped)
					if closeShim {
						shim.Close()
					} else {
						cancel()
					}
					<-done
					shim.bridges.Wait()
				}()
				select {
				case <-stopped:
				case <-time.After(time.Second):
					t.Fatal("cancellation left a control upgrade running")
				}
				if blockDial {
					select {
					case <-released:
					case <-time.After(time.Second):
						t.Fatal("upstream request connection left open")
					}
				}
				if got := calls.Load(); got != 2 {
					t.Fatalf("attempts = %d, want 2 before cancellation", got)
				}
			})
		}
	}
}

func TestShimControlRetryBudget(t *testing.T) {
	var calls atomic.Int32
	front := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		http.Error(w, "still restarting", 503)
	}))
	defer front.Close()
	shim := newTestShim(t, front.URL, key.NewMachine().Public())
	ctx, cancel := context.WithTimeout(t.Context(), 18*time.Second)
	defer cancel()
	start := time.Now()
	resp, err := controlUpgradeRequest(ctx, shim, "/ts2021")
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
	elapsed := time.Since(start)
	if resp.StatusCode != 502 || elapsed < 14*time.Second || elapsed > 17*time.Second {
		t.Fatalf("status = %d, duration = %s; want bounded aggregate 15 s budget", resp.StatusCode, elapsed)
	}
	if got := calls.Load(); got < 2 || got > 80 {
		t.Fatalf("attempts = %d; retries missing or unbounded rate", got)
	}
	t.Logf("aggregate duration = %s, upstream attempts = %d", elapsed, calls.Load())
}

func TestShimDERPDoesNotRetryRefusal(t *testing.T) {
	var calls atomic.Int32
	front := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		http.Error(w, "refused", 503)
	}))
	defer front.Close()
	shim := newTestShim(t, front.URL, key.NewMachine().Public())
	ctx, cancel := context.WithTimeout(t.Context(), time.Second)
	defer cancel()
	resp, err := controlUpgradeRequest(ctx, shim, "/derp")
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
	if resp.StatusCode != 502 || calls.Load() != 1 {
		t.Fatalf("status = %d, DERP attempts = %d", resp.StatusCode, calls.Load())
	}
}

func TestShimControlRejectsUntrustedCertificate(t *testing.T) {
	var attempts atomic.Int32
	front := httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		t.Error("untrusted TLS request reached HTTP handler")
	}))
	front.Config.ErrorLog = log.New(io.Discard, "", 0)
	front.Config.ConnState = func(_ net.Conn, state http.ConnState) {
		if state == http.StateNew {
			attempts.Add(1)
		}
	}
	front.StartTLS()
	defer front.Close()
	shim := newTestShim(t, front.URL, key.NewMachine().Public())
	ctx, cancel := context.WithTimeout(t.Context(), time.Second)
	defer cancel()
	resp, err := controlUpgradeRequest(ctx, shim, "/ts2021")
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
	if resp.StatusCode != 502 || attempts.Load() != 1 {
		t.Fatalf("status = %d, TLS attempts = %d", resp.StatusCode, attempts.Load())
	}
}

func TestShimControlRejectsInvalidWebSocketUpgrade(t *testing.T) {
	for _, protocol := range []string{"other-protocol", controlhttpcommon.UpgradeHeaderValue} {
		t.Run(protocol, func(t *testing.T) {
			var calls atomic.Int32
			front := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				calls.Add(1)
				if protocol == "other-protocol" {
					r.Header.Set("Sec-WebSocket-Protocol", protocol)
					c, err := websocket.Accept(w, r, &websocket.AcceptOptions{Subprotocols: []string{protocol}})
					if err == nil {
						c.CloseNow()
					}
					return
				}
				c, rw, err := w.(http.Hijacker).Hijack()
				if err == nil {
					defer c.Close()
					fmt.Fprintf(rw, "HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: invalid\r\nSec-WebSocket-Protocol: %s\r\n\r\n", protocol)
					rw.Flush()
				}
			}))
			defer front.Close()
			shim := newTestShim(t, front.URL, key.NewMachine().Public())
			ctx, cancel := context.WithTimeout(t.Context(), time.Second)
			defer cancel()
			resp, err := controlUpgradeRequest(ctx, shim, "/ts2021")
			if err != nil {
				t.Fatal(err)
			}
			resp.Body.Close()
			if resp.StatusCode != 502 || calls.Load() != 1 {
				t.Fatalf("status = %d, attempts = %d", resp.StatusCode, calls.Load())
			}
		})
	}
}

func TestShimControlDoesNotRetryNoiseFailure(t *testing.T) {
	for _, wrongPin := range []bool{false, true} {
		t.Run(fmt.Sprintf("wrong_pin=%t", wrongPin), func(t *testing.T) {
			var calls atomic.Int32
			machine := key.NewMachine()
			front := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				calls.Add(1)
				c, err := controlhttpserver.AcceptHTTP(r.Context(), w, r, machine, nil)
				if err == nil {
					c.Close()
				}
			}))
			defer front.Close()
			pinned := machine.Public()
			if wrongPin {
				pinned = key.NewMachine().Public()
			}
			shim := newTestShim(t, front.URL, pinned)
			ctx, cancel := context.WithTimeout(t.Context(), 300*time.Millisecond)
			defer cancel()
			if err := noiseEcho(ctx, shim, pinned); err == nil {
				t.Fatal("closed or unauthenticated Noise stream succeeded")
			}
			if got := calls.Load(); got != 1 {
				t.Fatalf("attempts = %d after Noise failure, want 1", got)
			}
		})
	}
}

func TestTransientControlUpgradeTransportErrors(t *testing.T) {
	for _, tc := range []struct {
		name string
		err  error
		want bool
	}{
		{"EOF", io.EOF, true},
		{"partial HTTP response", io.ErrUnexpectedEOF, true},
		{"connection refused", errControlConnRefused, true},
		{"connection reset", errControlConnReset, true},
		{"broken pipe", errControlBrokenPipe, true},
		{"deadline", context.DeadlineExceeded, false},
		{"canceled", context.Canceled, false},
		{"DNS", &net.DNSError{IsTemporary: true}, false},
		{"certificate", x509.UnknownAuthorityError{}, false},
		{"configuration", &net.AddrError{Err: "invalid address"}, false},
		{"protocol", errors.New("invalid WebSocket handshake"), false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			err := fmt.Errorf("WebSocket dial: %w", &url.Error{Op: "Get", URL: "http://127.0.0.1/ts2021", Err: tc.err})
			if got := transientControlUpgrade(nil, err); got != tc.want {
				t.Fatalf("retry = %t, want %t", got, tc.want)
			}
			if transientControlUpgrade(&http.Response{StatusCode: 101}, err) {
				t.Fatal("retried a response that switched protocols")
			}
		})
	}
}
