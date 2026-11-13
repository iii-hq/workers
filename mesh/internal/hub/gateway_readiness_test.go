package hub

import (
	"context"
	"fmt"
	"testing"
	"time"

	"github.com/iii-hq/workers/mesh/internal/join"
	"github.com/iii-hq/workers/mesh/internal/meshnet"
	"tailscale.com/net/packet"
	"tailscale.com/tsnet"
)

func gatewayPeerPath(ctx context.Context, peer *tsnet.Server, address string) error {
	// Up confirms control readiness. Probe the gateway's actual transport:
	// a single lost TSMP pong can leave Ping waiting until the whole test ends.
	conn, err := peer.Dial(ctx, "tcp", address)
	if err != nil {
		return fmt.Errorf("gateway peer %q encrypted TCP path to %s: %w", peer.Hostname, address, err)
	}
	return conn.Close()
}

func TestGatewayPeerPathWithLostTSMPPong(t *testing.T) {
	ctx, cancel := context.WithTimeout(t.Context(), 35*time.Second)
	defer cancel()
	cfg := DefaultConfig()
	cfg.Listen = "127.0.0.1:0"
	h, err := Start(ctx, Options{DataDir: t.TempDir()}, cfg, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer h.Close()
	cfg = h.Config()
	cfg.PublicURL = h.FrontURL()
	if err := h.ApplyConfig(ctx, cfg); err != nil {
		t.Fatal(err)
	}
	shim, err := meshnet.NewShim(h.FrontURL(), h.Coordinator.Public())
	if err != nil {
		t.Fatal(err)
	}
	defer shim.Close()
	peer := &tsnet.Server{Dir: t.TempDir(), Hostname: "loss-probe", ControlURL: shim.URL(), AuthKey: join.Secret(h.Coordinator.Identity(), h.Coordinator.Code())}
	defer peer.Close()
	if _, err := peer.Up(ctx); err != nil {
		t.Fatal(err)
	}
	listener, err := h.Coordinator.ListenTCP(50334)
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()
	h.Coordinator.SetGatewayPort(50334) // the hub node takes TCP only on its gateway port
	accepted := make(chan error, 1)
	go func() {
		conn, err := listener.Accept()
		if err == nil {
			err = conn.Close()
		}
		accepted <- err
	}()

	// Install before issuing any pings. This pinned SDK test hook models lost
	// TSMP replies without dropping TCP or changing the real userspace path.
	dropped := make(chan struct{}, 1)
	tun := tsnet.TestHooks.LocalBackend(peer).Sys().Tun.Get()
	tun.OnTSMPPongReceived = func(packet.TSMPPongReply) {
		select {
		case dropped <- struct{}{}:
		default:
		}
	}
	readyCtx, stopReady := context.WithCancel(ctx)
	defer stopReady()
	ready := make(chan error, 1)
	go func() { ready <- gatewayPeerPath(readyCtx, peer, listener.Addr().String()) }()
	select {
	case err := <-ready:
		if err != nil {
			t.Fatal(err)
		}
	case <-dropped:
		stopReady()
		<-ready
		t.Fatal("gateway readiness depends on a dropped TSMP pong")
	case <-ctx.Done():
		t.Fatal("gateway TCP path did not become ready")
	}
	select {
	case err := <-accepted:
		if err != nil {
			t.Fatal(err)
		}
	case <-ctx.Done():
		t.Fatal("readiness returned without a real TCP connection")
	}
}
