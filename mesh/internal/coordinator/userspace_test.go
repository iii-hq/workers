package coordinator

import (
	"bufio"
	"context"
	"crypto/rand"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"net/netip"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/iii-hq/workers/mesh/internal/meshnet"
	"github.com/iii-hq/workers/mesh/internal/relay"
)

func TestUserspacePrunesRetiredListeners(t *testing.T) {
	u := &userspace{}
	for range 20 {
		raw, err := net.Listen("tcp", "127.0.0.1:0")
		if err != nil {
			t.Fatal(err)
		}
		u.mu.Lock()
		ln := u.ownListenerLocked(raw)
		u.mu.Unlock()
		if err := ln.Close(); err != nil {
			t.Fatal(err)
		}
		u.mu.Lock()
		retained := len(u.listeners)
		u.mu.Unlock()
		if retained != 0 {
			t.Fatalf("retained retired listeners: %d", retained)
		}
	}
	raw, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	u.mu.Lock()
	ln := u.ownListenerLocked(raw)
	u.mu.Unlock()
	var wg sync.WaitGroup
	wg.Go(func() { u.close() })
	wg.Go(func() { ln.Close() })
	wg.Wait()
	if c, err := net.Dial("tcp", ln.Addr().String()); err == nil {
		c.Close()
		t.Fatal("shutdown left listener open")
	}
}

// Security: the hub is a client's only peer and must not reach the client host's services through the client's netstack.
func TestHubCannotReachClientHostLoopback(t *testing.T) {
	c := newCoordinator(t, t.TempDir())
	admission := httptest.NewServer(relay.AdmitHandler(c.Known))
	t.Cleanup(admission.Close)
	derp := relay.NewWithKey(c.RelayKey(), admission.URL)
	t.Cleanup(func() { derp.Close() })
	mux := http.NewServeMux()
	c.Handle(mux)
	relay.Handle(mux, derp)
	front := httptest.NewServer(mux)
	t.Cleanup(front.Close)
	shim, err := meshnet.NewShim(front.URL, c.Public())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { shim.Close() })
	ls, err := c.Listen(t.Context(), shim.URL(), 49135)
	if err != nil {
		t.Fatal(err)
	}
	c.SetGatewayPort(49135)
	service, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer service.Close()
	datagrams, err := net.ListenPacket("udp4", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer datagrams.Close()
	accepted := make(chan struct{}, 1)
	go func() {
		if conn, err := service.Accept(); err == nil {
			conn.Close()
			accepted <- struct{}{}
		}
	}()
	client, err := meshnet.UpUserspace(t.TempDir(), front.URL, rand.Text(), "laptop", c.Public())
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	for deadline := time.Now().Add(10 * time.Second); ; time.Sleep(10 * time.Millisecond) {
		if port, up := client.GatewayPort(); client.Self().IsValid() && up && port == 49135 {
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("timed out waiting for the client's address and gateway")
		}
	}
	// The forward proves the path between hub and client, so the refusal below is not a timeout.
	forward, token, err := client.Forward()
	if err != nil {
		t.Fatal(err)
	}
	local, err := net.Dial("tcp", strings.TrimPrefix(forward, "ws://"))
	if err != nil {
		t.Fatal(err)
	}
	defer local.Close()
	local.SetDeadline(time.Now().Add(30 * time.Second))
	req, _ := http.NewRequest("GET", forward, nil)
	req.Header.Set(meshnet.TokenHeader, token)
	if err := req.Write(local); err != nil {
		t.Fatal(err)
	}
	remote, err := ls[0].Accept()
	if err != nil {
		t.Fatal(err)
	}
	defer remote.Close()
	remote.SetDeadline(time.Now().Add(30 * time.Second))
	if _, err := http.ReadRequest(bufio.NewReader(remote)); err != nil {
		t.Fatalf("hub read: %v", err)
	}
	got := make([]byte, 2)
	if _, err := remote.Write([]byte("ok")); err != nil {
		t.Fatal(err)
	}
	if _, err := io.ReadFull(local, got); err != nil || string(got) != "ok" {
		t.Fatalf("client read %q: %v", got, err)
	}
	// Long enough for magicsock to fall back to DERP when a direct path drops data.
	ctx, cancel := context.WithTimeout(t.Context(), 30*time.Second)
	defer cancel()
	stack := c.backend.stack
	port := uint16(service.Addr().(*net.TCPAddr).Port)
	if conn, err := stack.DialContextTCP(ctx, netip.AddrPortFrom(client.Self(), port)); err == nil {
		conn.Close()
		t.Fatal("hub reached a 127.0.0.1 service on the client")
	} else if !strings.Contains(err.Error(), "refused") {
		t.Fatalf("want connection refused, got %v", err)
	}
	// Undenied UDP to a local address is forwarded to 127.0.0.1:port.
	udp, err := stack.DialContextUDP(ctx, netip.AddrPortFrom(client.Self(), uint16(datagrams.LocalAddr().(*net.UDPAddr).Port)))
	if err != nil {
		t.Fatal(err)
	}
	defer udp.Close()
	if _, err := udp.Write([]byte("x")); err != nil {
		t.Fatal(err)
	}
	datagrams.SetReadDeadline(time.Now().Add(time.Second))
	if _, _, err := datagrams.ReadFrom(make([]byte, 1)); err == nil {
		t.Fatal("hub reached a 127.0.0.1 UDP service on the client")
	}
	select {
	case <-accepted:
		t.Fatal("loopback service accepted the hub")
	default:
	}
}
