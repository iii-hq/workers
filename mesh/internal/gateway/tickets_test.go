package gateway

import (
	"encoding/hex"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

func TestTicketSingleUseAndExpiry(t *testing.T) {
	now := time.Now()
	tickets := NewTickets(10*time.Second, func() time.Time { return now })
	ticket := tickets.Mint("laptop")
	if raw, err := hex.DecodeString(ticket); err != nil || len(raw) != 32 {
		t.Fatalf("ticket entropy: %d %v", len(raw), err)
	}
	if node, ok := tickets.Consume(ticket); !ok || node != "laptop" {
		t.Fatalf("consume: %q %v", node, ok)
	}
	if _, ok := tickets.Consume(ticket); ok {
		t.Fatal("reused ticket")
	}
	expired := tickets.Mint("laptop")
	now = now.Add(11 * time.Second)
	if _, ok := tickets.Consume(expired); ok {
		t.Fatal("expired ticket accepted")
	}
	boundary := tickets.Mint("laptop")
	now = now.Add(10 * time.Second)
	if _, ok := tickets.Consume(boundary); ok {
		t.Fatal("expiry boundary accepted")
	}
}

func TestTicketConcurrentConsume(t *testing.T) {
	tickets := NewTickets(10*time.Second, time.Now)
	ticket := tickets.Mint("laptop")
	var wins atomic.Int32
	var wg sync.WaitGroup
	for range 20 {
		wg.Go(func() {
			if _, ok := tickets.Consume(ticket); ok {
				wins.Add(1)
			}
		})
	}
	wg.Wait()
	if wins.Load() != 1 {
		t.Fatalf("successful consumers=%d", wins.Load())
	}
}

func TestTicketsPruneExpiredAndClosedRecords(t *testing.T) {
	now := time.Now()
	tickets := NewTickets(10*time.Second, func() time.Time { return now })
	for range 20 {
		tickets.Mint("laptop")
	}
	now = now.Add(11 * time.Second)
	value := tickets.Mint("laptop")
	if len(tickets.pending) != 1 {
		t.Fatalf("expired records retained: %d", len(tickets.pending))
	}
	_, owner, ok := tickets.ConsumeOwned(value)
	if !ok {
		t.Fatal("fresh ticket missing")
	}
	owner.close()
	if len(tickets.pending) != 0 {
		t.Fatalf("consumed records retained: %d", len(tickets.pending))
	}
	_, unused := tickets.mint("laptop")
	unused.close()
	if len(tickets.pending) != 0 {
		t.Fatalf("closed records retained: %d", len(tickets.pending))
	}
}
