package hub

import (
	"crypto/rand"
	"encoding/hex"
	"sync"

	"github.com/iii-hq/workers/mesh/internal/gateway"
)

type Sessions struct {
	mu    sync.RWMutex
	nodes map[string]string
}

func (s *Sessions) Open(node string) string {
	var raw [32]byte
	if _, err := rand.Read(raw[:]); err != nil {
		panic(err)
	}
	key := hex.EncodeToString(raw[:])
	s.mu.Lock()
	if s.nodes == nil {
		s.nodes = make(map[string]string)
	}
	s.nodes[key] = node
	s.mu.Unlock()
	return key
}
func (s *Sessions) Valid(key string) (string, bool) {
	if s == nil {
		return "", false
	}
	s.mu.RLock()
	defer s.mu.RUnlock()
	node, ok := s.nodes[key]
	return node, ok
}
func (s *Sessions) CloseNode(node string) {
	if s == nil {
		return
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	for key, n := range s.nodes {
		if n == node {
			delete(s.nodes, key)
		}
	}
}
func (s *Sessions) closeAll() {
	if s == nil {
		return
	}
	s.mu.Lock()
	clear(s.nodes)
	s.mu.Unlock()
}
func (s *Sessions) openOwned(node string, owner *gateway.Connection) (string, bool) {
	key := s.Open(node)
	live := owner.OnClose(func() { s.mu.Lock(); delete(s.nodes, key); s.mu.Unlock() })
	return key, live
}
