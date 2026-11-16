package gateway

import (
	"crypto/rand"
	"encoding/hex"
	"sync"
	"time"
)

const TicketHeader = "x-iii-mesh-ticket"

type ticket struct {
	node    string
	expires time.Time
	owner   *Connection
}

type Tickets struct {
	mu      sync.Mutex
	ttl     time.Duration
	now     func() time.Time
	pending map[string]ticket
}

func NewTickets(ttl time.Duration, now func() time.Time) *Tickets {
	return &Tickets{ttl: ttl, now: now, pending: make(map[string]ticket)}
}
func (t *Tickets) Mint(node string) string { value, _ := t.mint(node); return value }
func (t *Tickets) mint(node string) (string, *Connection) {
	var raw [32]byte
	if _, err := rand.Read(raw[:]); err != nil {
		panic(err)
	}
	value := hex.EncodeToString(raw[:])
	owner := &Connection{}
	t.mu.Lock()
	now := t.now()
	t.prune(now)
	t.pending[value] = ticket{node, now.Add(t.ttl), owner}
	t.mu.Unlock()
	owner.OnClose(func() { t.mu.Lock(); delete(t.pending, value); t.mu.Unlock() })
	return value, owner
}
func (t *Tickets) Consume(value string) (string, bool) {
	node, _, ok := t.ConsumeOwned(value)
	return node, ok
}

// ConsumeOwned transfers the single-use ticket's connection identity to auth.
// Insert the session before registering its removal with Connection.OnClose.
func (t *Tickets) ConsumeOwned(value string) (string, *Connection, bool) {
	t.mu.Lock()
	defer t.mu.Unlock()
	now := t.now()
	t.prune(now)
	record, ok := t.pending[value]
	delete(t.pending, value)
	if !ok {
		return "", nil, false
	}
	record.owner.mu.Lock()
	closed := record.owner.closed
	record.owner.mu.Unlock()
	if closed {
		return "", nil, false
	}
	return record.node, record.owner, true
}
func (t *Tickets) prune(now time.Time) {
	for value, record := range t.pending {
		if !now.Before(record.expires) {
			delete(t.pending, value)
		}
	}
}

// Connection is the lifetime of one gateway request, including its upgrade.
// Callbacks execute outside internal locks and may call gateway/ticket APIs.
type Connection struct {
	mu        sync.Mutex
	closed    bool
	callbacks []func()
}

// OnClose registers fn until closure. If already closed, fn runs immediately
// and false is returned, allowing auth to refuse publishing a dead session.
func (c *Connection) OnClose(fn func()) bool {
	c.mu.Lock()
	if c.closed {
		c.mu.Unlock()
		fn()
		return false
	}
	c.callbacks = append(c.callbacks, fn)
	c.mu.Unlock()
	return true
}
func (c *Connection) close() {
	c.mu.Lock()
	if c.closed {
		c.mu.Unlock()
		return
	}
	c.closed = true
	callbacks := c.callbacks
	c.callbacks = nil
	c.mu.Unlock()
	for _, fn := range callbacks {
		fn()
	}
}
