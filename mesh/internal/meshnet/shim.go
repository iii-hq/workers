package meshnet

import (
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/sha256"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"math/big"
	"net"
	"net/http"
	"net/url"
	"strings"
	"sync"
	"time"

	"github.com/coder/websocket"
	"tailscale.com/control/controlhttp/controlhttpcommon"
	"tailscale.com/tailcfg"
	"tailscale.com/types/key"
)

const (
	derpURLHeader  = "X-Iii-Mesh-Derp-Url"
	derpCertHeader = "X-Iii-Mesh-Derp-Cert"
)

type Shim struct {
	public                        *url.URL
	pinned                        key.MachinePublic
	controlURL, derpURL, derpCert string
	control, relay                *http.Server
	cancel                        context.CancelFunc
	ctx                           context.Context
	mu                            sync.Mutex
	closed                        bool
	conns                         map[net.Conn]struct{}
	bridges                       sync.WaitGroup
	once                          sync.Once
	err                           error
}

func NewShim(publicURL string, pinned key.MachinePublic) (_ *Shim, err error) {
	u, err := url.Parse(publicURL)
	if err != nil {
		return nil, fmt.Errorf("public URL: %w", err)
	}
	if u.Hostname() == "" || u.User != nil || u.RawQuery != "" || u.ForceQuery || strings.Contains(publicURL, "#") || (u.Scheme != "https" && u.Scheme != "http" && u.Scheme != "wss" && u.Scheme != "ws") {
		return nil, errors.New("public URL must be an HTTP or WebSocket URL without credentials, query or fragment")
	}
	if pinned.IsZero() {
		return nil, errors.New("missing pinned control key")
	}
	cert, pin, err := shimCertificate()
	if err != nil {
		return nil, err
	}
	control, err := net.Listen("tcp4", "127.0.0.1:0")
	if err != nil {
		return nil, err
	}
	relay, err := net.Listen("tcp4", "127.0.0.1:0")
	if err != nil {
		control.Close()
		return nil, err
	}
	ctx, cancel := context.WithCancel(context.Background())
	s := &Shim{public: u, pinned: pinned, controlURL: "http://" + control.Addr().String(), derpURL: "https://" + relay.Addr().String() + "/derp", derpCert: pin, ctx: ctx, cancel: cancel, conns: make(map[net.Conn]struct{})}
	mux := http.NewServeMux()
	mux.HandleFunc("GET /key", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		json.NewEncoder(w).Encode(tailcfg.OverTLSPublicKeyResponse{PublicKey: s.pinned})
	})
	mux.HandleFunc("GET /ts2021", s.bridge)
	mux.HandleFunc("POST /ts2021", s.bridge)
	mux.HandleFunc("GET /derp", s.bridge)
	relayMux := http.NewServeMux()
	relayMux.HandleFunc("GET /derp", s.bridge)
	for _, path := range []string{"/derp/probe", "/derp/latency-check"} {
		relayMux.HandleFunc("GET "+path, func(w http.ResponseWriter, r *http.Request) {
			w.Header().Set("Access-Control-Allow-Origin", "*")
			w.WriteHeader(http.StatusNoContent)
		})
	}
	s.control = &http.Server{Handler: mux, ReadHeaderTimeout: 10 * time.Second}
	s.relay = &http.Server{Handler: relayMux, ReadHeaderTimeout: 10 * time.Second, ErrorLog: log.New(io.Discard, "", 0)}
	go s.control.Serve(control)
	go s.relay.Serve(tls.NewListener(relay, &tls.Config{Certificates: []tls.Certificate{cert}, MinVersion: tls.VersionTLS12}))
	return s, nil
}

func (s *Shim) URL() string { return s.controlURL }

func (s *Shim) Close() error {
	s.once.Do(func() {
		s.mu.Lock()
		s.closed = true
		s.cancel()
		for c := range s.conns {
			c.Close()
		}
		s.mu.Unlock()
		s.err = errors.Join(s.control.Close(), s.relay.Close())
		s.bridges.Wait()
	})
	return s.err
}

func (s *Shim) bridge(w http.ResponseWriter, r *http.Request) {
	protocol := "derp"
	if r.URL.Path == "/ts2021" {
		protocol = controlhttpcommon.UpgradeHeaderValue
	}
	if !strings.EqualFold(r.Header.Get("Upgrade"), protocol) || !headerToken(r.Header.Get("Connection"), "upgrade") {
		http.Error(w, "native protocol upgrade required", http.StatusBadRequest)
		return
	}
	s.mu.Lock()
	if s.closed {
		s.mu.Unlock()
		http.Error(w, "shim closed", http.StatusServiceUnavailable)
		return
	}
	s.bridges.Add(1)
	s.mu.Unlock()
	defer s.bridges.Done()
	ctx, cancel := context.WithCancel(r.Context())
	defer cancel()
	stop := context.AfterFunc(s.ctx, cancel)
	defer stop()
	target := s.public.JoinPath(r.URL.Path)
	if target.Scheme == "http" {
		target.Scheme = "ws"
	}
	if target.Scheme == "https" {
		target.Scheme = "wss"
	}
	headers := make(http.Header)
	if r.URL.Path == "/ts2021" {
		handshake := r.Header.Get(controlhttpcommon.HandshakeHeaderName)
		if handshake == "" {
			http.Error(w, "missing control handshake", http.StatusBadRequest)
			return
		}
		query := target.Query()
		query.Set(controlhttpcommon.HandshakeHeaderName, handshake)
		target.RawQuery = query.Encode()
		headers.Set(derpURLHeader, s.derpURL)
		headers.Set(derpCertHeader, s.derpCert)
	}
	dialCtx, dialCancel := context.WithTimeout(ctx, 15*time.Second)
	ws, err := dialFrontDoor(dialCtx, target.String(), &websocket.DialOptions{HTTPHeader: headers, Subprotocols: []string{protocol}, CompressionMode: websocket.CompressionDisabled}, r.URL.Path == "/ts2021")
	dialCancel()
	if err != nil {
		http.Error(w, "front door upgrade failed", http.StatusBadGateway)
		return
	}
	defer ws.CloseNow()
	if ws.Subprotocol() != protocol {
		http.Error(w, "front door protocol mismatch", http.StatusBadGateway)
		return
	}
	remote := websocket.NetConn(ctx, ws, websocket.MessageBinary)
	defer remote.Close()
	h, ok := w.(http.Hijacker)
	if !ok {
		http.Error(w, "HTTP/1 required", http.StatusBadRequest)
		return
	}
	local, rw, err := h.Hijack()
	if err != nil {
		return
	}
	defer local.Close()
	s.mu.Lock()
	if s.closed {
		s.mu.Unlock()
		return
	}
	s.conns[local] = struct{}{}
	s.mu.Unlock()
	defer func() { s.mu.Lock(); delete(s.conns, local); s.mu.Unlock() }()
	if _, err := fmt.Fprintf(rw, "HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: %s\r\n\r\n", r.Header.Get("Upgrade")); err != nil {
		return
	}
	if err := rw.Flush(); err != nil {
		return
	}
	done := make(chan struct{})
	go func() { defer close(done); io.Copy(remote, rw); remote.Close(); local.Close() }()
	io.Copy(local, remote)
	local.Close()
	remote.Close()
	<-done
}

func dialFrontDoor(ctx context.Context, target string, options *websocket.DialOptions, retryControl bool) (*websocket.Conn, error) {
	// A transient 502 at the native HTTP-only control dialer stalls it until
	// its deadline. Recover before upgrading, while no Noise bytes have flowed.
	backoff := 25 * time.Millisecond
	for {
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		ws, resp, err := websocket.Dial(ctx, target, options)
		if err == nil {
			return ws, nil
		}
		if ws != nil {
			ws.CloseNow()
		}
		if resp != nil && resp.Body != nil {
			resp.Body.Close()
		}
		if !retryControl || !transientControlUpgrade(resp, err) {
			return nil, err
		}
		timer := time.NewTimer(backoff)
		select {
		case <-ctx.Done():
			timer.Stop()
			return nil, ctx.Err()
		case <-timer.C:
		}
		backoff = min(2*backoff, 250*time.Millisecond)
	}
}

func transientControlUpgrade(resp *http.Response, err error) bool {
	if resp != nil {
		switch resp.StatusCode {
		case http.StatusBadGateway, http.StatusServiceUnavailable, http.StatusGatewayTimeout:
			return true
		}
		return false
	}
	return errors.Is(err, io.EOF) || errors.Is(err, io.ErrUnexpectedEOF) ||
		errors.Is(err, errControlConnRefused) || errors.Is(err, errControlConnReset) || errors.Is(err, errControlBrokenPipe)
}

func headerToken(value, token string) bool {
	for _, v := range strings.Split(value, ",") {
		if strings.EqualFold(strings.TrimSpace(v), token) {
			return true
		}
	}
	return false
}

func shimCertificate() (tls.Certificate, string, error) {
	pub, private, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		return tls.Certificate{}, "", err
	}
	serial, err := rand.Int(rand.Reader, new(big.Int).Lsh(big.NewInt(1), 128))
	if err != nil {
		return tls.Certificate{}, "", err
	}
	template := &x509.Certificate{SerialNumber: serial, Subject: pkix.Name{CommonName: "iii mesh loopback relay"}, NotBefore: time.Now().Add(-time.Minute), NotAfter: time.Now().AddDate(10, 0, 0), KeyUsage: x509.KeyUsageDigitalSignature, ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth}, IPAddresses: []net.IP{net.ParseIP("127.0.0.1")}}
	raw, err := x509.CreateCertificate(rand.Reader, template, template, pub, private)
	if err != nil {
		return tls.Certificate{}, "", err
	}
	return tls.Certificate{Certificate: [][]byte{raw}, PrivateKey: private}, fmt.Sprintf("sha256-raw:%x", sha256.Sum256(raw)), nil
}
