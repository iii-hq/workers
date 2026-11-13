package hub

import (
	"context"
	"net"
	"strconv"
	"testing"
	"time"

	"tailscale.com/net/stun"
)

func TestSTUNReloadKeepsLastGoodListener(t *testing.T) {
	ctx, cancel := context.WithTimeout(t.Context(), 20*time.Second)
	defer cancel()
	cfg := DefaultConfig()
	cfg.Listen = "127.0.0.1:0"
	cfg.PublicURL = "http://127.0.0.1"
	h, err := Start(ctx, Options{DataDir: t.TempDir()}, cfg, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer h.Close()
	occupied, err := net.ListenUDP("udp", &net.UDPAddr{Port: 0})
	if err != nil {
		t.Fatal(err)
	}
	defer occupied.Close()
	bad := h.Config()
	bad.StunPort = occupied.LocalAddr().(*net.UDPAddr).Port
	bad.GatewayPort = 50250
	if err := h.ApplyConfig(ctx, bad); err == nil {
		t.Fatal("occupied STUN port accepted")
	}
	if h.Config().StunPort != 0 || h.Config().GatewayPort != 49135 {
		t.Fatal("failed STUN bind changed config")
	}
	// The gateway staged before the failed UDP bind must also be released.
	l, err := h.Coordinator.ListenTCP(50250)
	if err != nil {
		t.Fatal(err)
	}
	l.Close()
	probe, err := net.ListenUDP("udp", &net.UDPAddr{Port: 0})
	if err != nil {
		t.Fatal(err)
	}
	port := probe.LocalAddr().(*net.UDPAddr).Port
	probe.Close()
	cfg = h.Config()
	cfg.StunPort = port
	if err := h.ApplyConfig(ctx, cfg); err != nil {
		t.Fatal(err)
	}
	conn, err := net.Dial("udp", net.JoinHostPort("127.0.0.1", strconv.Itoa(port)))
	if err != nil {
		t.Fatal(err)
	}
	defer conn.Close()
	conn.SetDeadline(time.Now().Add(time.Second))
	id := stun.NewTxID()
	if _, err := conn.Write(stun.Request(id)); err != nil {
		t.Fatal(err)
	}
	var b [1500]byte
	n, err := conn.Read(b[:])
	if err != nil {
		t.Fatal(err)
	}
	got, addr, err := stun.ParseResponse(b[:n])
	if err != nil || got != id || !addr.Addr().IsLoopback() {
		t.Fatalf("STUN response: %v %v %v", got, addr, err)
	}
	cfg.StunPort = 0
	if err := h.ApplyConfig(ctx, cfg); err != nil {
		t.Fatal(err)
	}
	rebound, err := net.ListenUDP("udp", &net.UDPAddr{Port: port})
	if err != nil {
		t.Fatal(err)
	}
	rebound.Close()
}
