package coordinator

import (
	"net"
	"sync"
	"testing"
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
