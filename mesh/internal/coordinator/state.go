// MIT License
//
// Copyright (c) 2026 Alexander Yue
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

package coordinator

import (
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"encoding/base32"
	"encoding/json"
	"errors"
	"fmt"
	"net/netip"
	"os"
	"path/filepath"
	"slices"

	"tailscale.com/ipn"
	"tailscale.com/tailcfg"
	"tailscale.com/types/key"
)

type state struct {
	Identity        ed25519.PrivateKey      `json:"identity"`
	Control         key.MachinePrivate      `json:"control"`
	Relay           key.NodePrivate         `json:"relay"`
	Prefix          netip.Prefix            `json:"prefix"`
	CodeSeed        []byte                  `json:"code_seed"`
	Next            tailcfg.NodeID          `json:"next"`
	Nodes           []*node                 `json:"nodes"`
	RefusedNodes    []key.NodePublic        `json:"refused_nodes"`
	RefusedMachines []key.MachinePublic     `json:"refused_machines"`
	HubState        map[ipn.StateKey][]byte `json:"hub_state"`
}

func New(dir string, join func(secret, addr string) (string, error)) (*Coordinator, error) {
	if err := os.MkdirAll(dir, 0700); err != nil {
		return nil, err
	}
	c := &Coordinator{file: filepath.Join(dir, "state.json"), join: join, self: rand.Text(), wakes: map[chan struct{}]bool{}}
	b, err := os.ReadFile(c.file)
	switch {
	case errors.Is(err, os.ErrNotExist):
		_, id, err := ed25519.GenerateKey(rand.Reader)
		if err != nil {
			return nil, err
		}
		var addr [16]byte
		_, _ = rand.Read(addr[1:6])
		addr[0] = 0xfd
		seed := make([]byte, 32)
		_, _ = rand.Read(seed)
		c.st = state{Identity: id, Control: key.NewMachine(), Relay: key.NewNode(), Prefix: netip.PrefixFrom(netip.AddrFrom16(addr), 48), CodeSeed: seed, Next: 1, HubState: map[ipn.StateKey][]byte{}}
		if err := c.save(); err != nil {
			return nil, err
		}
	case err != nil:
		return nil, err
	default:
		if err := json.Unmarshal(b, &c.st); err != nil {
			return nil, fmt.Errorf("decode hub state: %w", err)
		}
		if err := c.validate(); err != nil {
			return nil, err
		}
		if err := os.Chmod(c.file, 0600); err != nil {
			return nil, err
		}
	}
	c.key = c.st.Control
	c.ctx, c.cancel = context.WithCancel(context.Background())
	for _, n := range c.st.Nodes {
		c.expire(n)
	}
	return c, nil
}

func (c *Coordinator) validate() error {
	st := &c.st
	if len(st.Identity) != ed25519.PrivateKeySize || !slices.Equal(ed25519.NewKeyFromSeed(st.Identity[:ed25519.SeedSize]), st.Identity) || st.Control.IsZero() || st.Relay.IsZero() || len(st.CodeSeed) != 32 || !st.Prefix.IsValid() || !st.Prefix.Addr().Is6() || st.Prefix.Bits() != 48 || st.Prefix.Addr().As16()[0] != 0xfd || st.Prefix.Masked() != st.Prefix || st.Next < 1 {
		return errors.New("invalid hub state")
	}
	ids, names, machines, keys := map[tailcfg.NodeID]bool{}, map[string]bool{}, map[key.MachinePublic]bool{}, map[key.NodePublic]bool{}
	for _, n := range st.Nodes {
		if n == nil || n.ID < 1 || n.ID >= st.Next || ids[n.ID] || SanitizeName(n.Name) != n.Name || n.Key.IsZero() || n.Machine.IsZero() || keys[n.Key] || machines[n.Machine] || !n.Removed && names[n.Name] || !slices.Contains(n.Keys, n.Key) {
			return errors.New("invalid node registry")
		}
		ids[n.ID], keys[n.Key], machines[n.Machine] = true, true, true
		if !n.Removed {
			names[n.Name] = true
		}
	}
	if st.HubState == nil {
		st.HubState = map[ipn.StateKey][]byte{}
	}
	return nil
}

func (c *Coordinator) Identity() ed25519.PrivateKey {
	c.mu.Lock()
	defer c.mu.Unlock()
	return slices.Clone(c.st.Identity)
}
func (c *Coordinator) RelayKey() key.NodePrivate { c.mu.Lock(); defer c.mu.Unlock(); return c.st.Relay }
func (c *Coordinator) Prefix() netip.Prefix      { c.mu.Lock(); defer c.mu.Unlock(); return c.st.Prefix }
func (c *Coordinator) Code() string {
	c.mu.Lock()
	defer c.mu.Unlock()
	code := base32.NewEncoding("0123456789ABCDEFGHJKMNPQRSTVWXYZ").WithPadding(base32.NoPadding).EncodeToString(c.st.CodeSeed[:5])
	return code[:4] + "-" + code[4:]
}

// StateStore keeps backend machine keys, node keys and profiles in the same
// commit as the coordinator identity and registry.
func (c *Coordinator) StateStore() ipn.StateStore { return hubStore{c} }

type hubStore struct{ c *Coordinator }

func (s hubStore) ReadState(id ipn.StateKey) ([]byte, error) {
	s.c.mu.Lock()
	defer s.c.mu.Unlock()
	b, ok := s.c.st.HubState[id]
	if !ok {
		return nil, ipn.ErrStateNotExist
	}
	return slices.Clone(b), nil
}
func (s hubStore) WriteState(id ipn.StateKey, b []byte) error {
	s.c.mu.Lock()
	defer s.c.mu.Unlock()
	old, existed := s.c.st.HubState[id]
	if b == nil {
		delete(s.c.st.HubState, id)
	} else {
		s.c.st.HubState[id] = slices.Clone(b)
	}
	if err := s.c.save(); err != nil {
		if existed {
			s.c.st.HubState[id] = old
		} else {
			delete(s.c.st.HubState, id)
		}
		return err
	}
	return nil
}

func (c *Coordinator) save() error {
	b, err := json.MarshalIndent(c.st, "", "  ")
	if err != nil {
		return err
	}
	f, err := os.CreateTemp(filepath.Dir(c.file), ".state-*")
	if err != nil {
		return err
	}
	defer os.Remove(f.Name())
	if err := f.Chmod(0600); err != nil {
		_ = f.Close()
		return err
	}
	if _, err = f.Write(b); err != nil {
		_ = f.Close()
		return err
	}
	if err = f.Sync(); err != nil {
		_ = f.Close()
		return err
	}
	if err = f.Close(); err != nil {
		return err
	}
	return os.Rename(f.Name(), c.file)
}
