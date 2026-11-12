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
	"crypto/subtle"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"math/rand/v2"
	"net/http"
	"net/netip"
	"net/url"
	"slices"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/iii-hq/workers/mesh/internal/join"
	"golang.org/x/net/http2"
	"tailscale.com/control/controlhttp/controlhttpserver"
	"tailscale.com/tailcfg"
	"tailscale.com/tstime"
	"tailscale.com/types/key"
	"tailscale.com/util/zstdframe"
)

const (
	keepAlive = 50 * time.Second
	jitter    = 9 * time.Second
	bodyLimit = 1 << 20
	user      = tailcfg.UserID(1)
	region    = 900
	project   = "iii.internal"
	idle      = 3 * time.Minute
	pingIdle  = time.Minute
)

type node struct {
	TokenID string    `json:"token_id,omitempty"`
	LastMap time.Time `json:"last_map,omitempty"`
	RuntimeMetadata
	ID        tailcfg.NodeID    `json:"id"`
	Name      string            `json:"name"`
	Machine   key.MachinePublic `json:"machine"`
	Key       key.NodePublic    `json:"key"`
	Keys      []key.NodePublic  `json:"keys"`
	Ephemeral bool              `json:"ephemeral,omitempty"`
	Removed   bool              `json:"removed,omitempty"`
	JoinedAt  time.Time         `json:"joined_at"`
	LastSeen  time.Time         `json:"last_seen"`
	Disco     key.DiscoPublic   `json:"disco"`
	Endpoints []netip.AddrPort  `json:"endpoints"`
	Hostinfo  *tailcfg.Hostinfo `json:"hostinfo"`
	capVer    tailcfg.CapabilityVersion
	streams   int
	ver       uint64
	timer     tstime.TimerController
}

type NodeInfo struct {
	RuntimeMetadata
	Ephemeral bool       `json:"ephemeral"`
	Name      string     `json:"name"`
	Addr      netip.Addr `json:"addr"`
	Online    bool       `json:"online"`
	JoinedAt  time.Time  `json:"joined_at"`
	LastSeen  time.Time  `json:"last_seen"`
}

type Coordinator struct {
	clock        tstime.Clock
	limit        *join.Limit
	key          key.MachinePrivate
	file         string
	join         func(secret, addr string) (string, error)
	self         string
	gone         func(name string)
	nodeObserver func(NodeChange)
	beforeRename func() error
	mu           sync.Mutex
	st           state
	host         string
	relayPort    int
	stunPort     int
	gatewayPort  int
	ver          uint64
	wakes        map[chan struct{}]bool
	taken        func(string) bool
	trusted      []netip.Prefix
	ctx          context.Context
	cancel       context.CancelFunc
	life         sync.Mutex
	backend      *userspace
}

func (c *Coordinator) Public() key.MachinePublic { return c.key.Public() }

func (c *Coordinator) Relay(u string) {
	parsed, err := url.Parse(u)
	if err != nil || parsed.Hostname() == "" {
		return
	}
	port := 443
	if parsed.Port() != "" {
		port, err = strconv.Atoi(parsed.Port())
		if err != nil || port < 1 || port > 65535 {
			return
		}
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	c.host, c.relayPort = parsed.Hostname(), port
	c.wake()
}

func (c *Coordinator) SetGone(gone func(string)) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.gone = gone
}

func (c *Coordinator) SetNameTaken(taken func(string) bool) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.taken = taken
}
func (c *Coordinator) SetTrustedProxies(trusted []netip.Prefix) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.trusted = slices.Clone(trusted)
}
func (c *Coordinator) Known(k key.NodePublic) bool {
	c.mu.Lock()
	defer c.mu.Unlock()
	n := c.byKey(k)
	return c.authorized(n)
}
func (c *Coordinator) Node(a netip.Addr) (string, bool) {
	c.mu.Lock()
	defer c.mu.Unlock()
	for _, n := range c.st.Nodes {
		if c.authorized(n) && c.address(n.ID) == a {
			return n.Name, true
		}
	}
	return "", false
}
func (c *Coordinator) Nodes() []NodeInfo {
	c.mu.Lock()
	defer c.mu.Unlock()
	nodes := make([]NodeInfo, 0, len(c.st.Nodes))
	for _, n := range c.st.Nodes {
		if c.authorized(n) {
			nodes = append(nodes, NodeInfo{RuntimeMetadata: n.RuntimeMetadata, Ephemeral: n.Ephemeral, Name: n.Name, Addr: c.address(n.ID), Online: n.streams > 0, JoinedAt: n.JoinedAt, LastSeen: n.LastSeen})
		}
	}
	return nodes
}

func (c *Coordinator) Remove(name string) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	n := c.byName(name)
	if n == nil || name == "hub" {
		return fmt.Errorf("no removable node %q", name)
	}
	before := c.st
	c.st.CodeSeed = newSeed()
	c.st.RefusedNodes = append(slices.Clone(c.st.RefusedNodes), n.Keys...)
	c.st.RefusedMachines = append(slices.Clone(c.st.RefusedMachines), n.Machine)
	c.revokeToken(n.TokenID)
	n.Removed = true
	if err := c.save(); err != nil {
		c.st = before
		n.Removed = false
		return err
	}
	c.changed(n)
	c.notifyNode(n, "removed")
	if c.gone != nil {
		go c.gone(n.Name)
	}
	return nil
}

func (c *Coordinator) Close() error {
	c.cancel()
	c.life.Lock()
	defer c.life.Unlock()
	if c.backend != nil {
		c.backend.close()
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	for _, n := range c.st.Nodes {
		if n.timer != nil {
			n.timer.Stop()
		}
	}
	return nil
}

func (c *Coordinator) expire(n *node) {
	if !n.Ephemeral || n.Removed || c.ctx.Err() != nil {
		return
	}
	if n.timer != nil {
		n.timer.Stop()
	}
	last := n.LastMap
	if last.IsZero() {
		last = n.JoinedAt
	}
	delay := max(time.Duration(0), idle-c.clock.Since(last))
	c.scheduleExpiry(n, delay)
}
func (c *Coordinator) scheduleExpiry(n *node, delay time.Duration) {
	n.timer = c.clock.AfterFunc(delay, func() {
		c.mu.Lock()
		defer c.mu.Unlock()
		if c.ctx.Err() != nil || n.Removed || n.streams > 0 || !slices.Contains(c.st.Nodes, n) {
			return
		}
		last := n.LastMap
		if last.IsZero() {
			last = n.JoinedAt
		}
		if c.clock.Since(last) < idle {
			c.expire(n)
			return
		}
		before := c.st.Nodes
		c.st.Nodes = slices.DeleteFunc(slices.Clone(c.st.Nodes), func(p *node) bool { return p == n })
		if err := c.save(); err != nil {
			c.st.Nodes = before
			log.Printf("coordinator: expire: %v", err)
			c.scheduleExpiry(n, time.Second)
			return
		}
		c.wake()
		c.notifyNode(n, "removed")
		if c.gone != nil {
			go c.gone(n.Name)
		}
	})
}

func (c *Coordinator) Handle(mux *http.ServeMux) {
	mux.Handle("GET /id", join.IdentityHandler(c.Identity(), c.Public()))
	mux.HandleFunc("GET /key", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(tailcfg.OverTLSPublicKeyResponse{PublicKey: c.Public()})
	})
	mux.HandleFunc("/ts2021", c.upgrade)
}

const (
	DERPURLHeader  = "X-Iii-Mesh-Derp-Url"
	DERPCertHeader = "X-Iii-Mesh-Derp-Cert"
)

type relayContextKey struct{}

func sessionRelay(header http.Header) (*tailcfg.DERPNode, error) {
	urls, certs := header.Values(DERPURLHeader), header.Values(DERPCertHeader)
	if len(urls) == 0 && len(certs) == 0 {
		return nil, nil
	}
	if len(urls) != 1 || len(certs) != 1 {
		return nil, errors.New("one relay URL and certificate pin required")
	}
	u, err := url.Parse(urls[0])
	if err != nil || u.Scheme != "https" || u.User != nil || u.Path != "/derp" || u.RawPath != "" || u.RawQuery != "" || u.ForceQuery || strings.ContainsRune(urls[0], '#') || u.RawFragment != "" {
		return nil, errors.New("invalid relay URL")
	}
	addr, err := netip.ParseAddr(u.Hostname())
	port, pErr := strconv.Atoi(u.Port())
	if err != nil || addr.Zone() != "" || !addr.Unmap().IsLoopback() || pErr != nil || port < 1 || port > 65535 {
		return nil, errors.New("relay must use a loopback IP and explicit port")
	}
	hash, ok := strings.CutPrefix(certs[0], "sha256-raw:")
	decoded, err := hex.DecodeString(hash)
	if !ok || err != nil || len(decoded) != 32 || hash != strings.ToLower(hash) {
		return nil, errors.New("invalid relay certificate pin")
	}
	n := &tailcfg.DERPNode{Name: "hub", RegionID: region, HostName: addr.String(), DERPPort: port, CertName: certs[0], STUNPort: -1, IPv4: "none", IPv6: "none"}
	if addr.Is4() {
		n.IPv4 = addr.String()
	} else {
		n.IPv6 = addr.String()
	}
	return n, nil
}

func (c *Coordinator) upgrade(w http.ResponseWriter, r *http.Request) {
	relay, err := sessionRelay(r.Header)
	if err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}
	ctx := r.Context()
	if relay != nil {
		ctx = context.WithValue(ctx, relayContextKey{}, relay)
	}
	c.mu.Lock()
	trusted := slices.Clone(c.trusted)
	c.mu.Unlock()
	addr := join.ClientAddr(r, trusted)
	conn, err := controlhttpserver.AcceptHTTP(r.Context(), w, r, c.key, nil)
	if err != nil {
		return
	}
	defer conn.Close()
	stop := context.AfterFunc(c.ctx, func() { _ = conn.Close() })
	defer stop()
	machine := conn.Peer()
	mux := http.NewServeMux()
	mux.HandleFunc("POST /machine/register", func(w http.ResponseWriter, r *http.Request) {
		var req tailcfg.RegisterRequest
		if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
			http.Error(w, err.Error(), http.StatusBadRequest)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(c.register(machine, addr, req))
	})
	mux.HandleFunc("POST /machine/map", func(w http.ResponseWriter, r *http.Request) { c.serveMap(w, r, machine) })
	mux.HandleFunc("POST "+renamePath, func(w http.ResponseWriter, r *http.Request) { c.serveRename(w, r, machine) })
	(&http2.Server{ReadIdleTimeout: pingIdle}).ServeConn(conn, &http2.ServeConnOpts{Context: ctx, BaseConfig: &http.Server{Handler: http.MaxBytesHandler(mux, bodyLimit)}})
}

func (c *Coordinator) register(machine key.MachinePublic, addr string, req tailcfg.RegisterRequest) (result tailcfg.RegisterResponse) {
	if machine.IsZero() || req.NodeKey.IsZero() {
		return tailcfg.RegisterResponse{Error: "missing machine or node key"}
	}
	// The join callback may reenter the coordinator; only enrollment unlocks.
	// Accepted machine authority stays locked through key rotation.
	c.mu.Lock()
	if c.ctx.Err() != nil {
		c.mu.Unlock()
		return tailcfg.RegisterResponse{Error: "coordinator closed"}
	}
	n := c.byKey(req.NodeKey)
	seed := c.st.CodeSeed
	needsJoin := n == nil && !c.authorized(c.byMachine(machine))
	limit := c.limit
	reserved := ""
	secret := ""
	if req.Auth != nil {
		secret = req.Auth.AuthKey
	}
	tokenAuth := join.IsTokenSecret(secret)
	if needsJoin {
		c.mu.Unlock()
		if limit != nil && secret != c.self {
			if err := limit.Allow(addr); err != nil {
				return tailcfg.RegisterResponse{Error: err.Error()}
			}
			defer func() {
				if result.Error != "" {
					limit.Refused(addr)
				}
			}()
		}
		if subtle.ConstantTimeCompare([]byte(secret), []byte(c.self)) == 1 {
			reserved = "hub"
		} else {
			if c.join == nil {
				return tailcfg.RegisterResponse{Error: "joining disabled"}
			}
			var err error
			reserved, err = c.join(secret, addr)
			if err != nil {
				return tailcfg.RegisterResponse{Error: err.Error()}
			}
			if reserved != "" && (SanitizeName(reserved) != reserved || isReserved(reserved)) {
				return tailcfg.RegisterResponse{Error: "invalid reserved name"}
			}
		}
		c.mu.Lock()
	}
	defer c.mu.Unlock()
	if c.ctx.Err() != nil {
		return tailcfg.RegisterResponse{Error: "coordinator closed"}
	}
	if needsJoin && !tokenAuth && !slices.Equal(seed, c.st.CodeSeed) {
		return tailcfg.RegisterResponse{Error: "join code changed during registration"}
	}
	if slices.Contains(c.st.RefusedNodes, req.NodeKey) || slices.Contains(c.st.RefusedMachines, machine) {
		return tailcfg.RegisterResponse{Error: "node keys refused"}
	}
	for _, owner := range c.st.Nodes {
		if owner.Machine != machine && slices.Contains(owner.Keys, req.NodeKey) {
			return tailcfg.RegisterResponse{Error: "node key registered to another machine"}
		}
	}
	n = c.byKey(req.NodeKey)
	if n != nil && n.Machine != machine {
		return tailcfg.RegisterResponse{Error: "node key registered to another machine"}
	}
	if n != nil && !c.authorized(n) {
		return tailcfg.RegisterResponse{Error: "node keys refused"}
	}
	if !req.Expiry.IsZero() && req.Expiry.Before(time.Now()) {
		if n != nil {
			before := c.st.Nodes
			c.st.Nodes = slices.DeleteFunc(slices.Clone(c.st.Nodes), func(p *node) bool { return p == n })
			if err := c.save(); err != nil {
				c.st.Nodes = before
				return tailcfg.RegisterResponse{Error: err.Error()}
			}
			if n.timer != nil {
				n.timer.Stop()
			}
			c.wake()
			c.notifyNode(n, "removed")
			if c.gone != nil {
				go c.gone(n.Name)
			}
		}
		return tailcfg.RegisterResponse{NodeKeyExpired: true}
	}
	if n == nil {
		before := c.st
		var old node
		n = c.byMachine(machine)
		existed := n != nil
		if existed {
			old = *n
			if !c.authorized(n) {
				return tailcfg.RegisterResponse{Error: "node keys refused"}
			}
		} else {
			tokenIndex := -1
			if tokenAuth {
				var err error
				tokenIndex, err = c.token(secret)
				if err != nil {
					return tailcfg.RegisterResponse{Error: err.Error()}
				}
			}
			name := reserved
			if name == "" {
				host := ""
				if req.Hostinfo != nil {
					host = req.Hostinfo.Hostname
				}
				name = AllocateName(host, func(s string) bool { return c.byName(s) != nil || c.taken != nil && c.taken(s) })
			}
			if c.byName(name) != nil {
				return tailcfg.RegisterResponse{Error: "reserved name is taken"}
			}
			n = &node{ID: c.st.Next, Name: name, Machine: machine, Ephemeral: req.Ephemeral}

			if tokenIndex >= 0 {
				token := c.st.Tokens[tokenIndex]
				n.TokenID, n.Ephemeral = token.ID, token.Ephemeral
				token.Nodes = append(slices.Clone(token.Nodes), name)
				c.st.Tokens = slices.Clone(c.st.Tokens)
				c.st.Tokens[tokenIndex] = token
			}
			c.st.Next++
			c.st.Nodes = append(slices.Clone(c.st.Nodes), n)
			// The code admits one machine; joined machines reconnect by key.
			if needsJoin && !tokenAuth && reserved != "hub" {
				c.st.CodeSeed = newSeed()
			}
		}
		n.Key, n.Hostinfo = req.NodeKey, req.Hostinfo.Clone()
		n.Keys = append(slices.Clone(n.Keys), req.NodeKey)
		n.recordActivity(c.clock.Now())
		if err := c.save(); err != nil {
			c.st = before
			if existed {
				*n = old
			}
			return tailcfg.RegisterResponse{Error: err.Error()}
		}
		c.changed(n)
		if !existed {
			c.notifyNode(n, "joined")
		}
		c.expire(n)
	} else {
		before := *n
		n.recordActivity(c.clock.Now())
		if err := c.save(); err != nil {
			*n = before
			return tailcfg.RegisterResponse{Error: err.Error()}
		}
	}
	return tailcfg.RegisterResponse{MachineAuthorized: c.authorized(n), User: tailcfg.User{ID: user, DisplayName: "iii mesh"}, Login: tailcfg.Login{ID: tailcfg.LoginID(user), LoginName: "iii mesh", DisplayName: "iii mesh"}}
}

func (c *Coordinator) address(id tailcfg.NodeID) netip.Addr {
	a := c.st.Prefix.Addr().As16()
	binary.BigEndian.PutUint64(a[8:], uint64(id))
	return netip.AddrFrom16(a)
}

// serveMap is headscale's PollNetMapHandler. A poll gets the whole map, then
// what changes, then a keepalive when nothing has for a while. A request
// that does not poll only updates the node.
func (c *Coordinator) serveMap(w http.ResponseWriter, r *http.Request, machine key.MachinePublic) {
	var req tailcfg.MapRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}
	c.mu.Lock()
	n := c.byKey(req.NodeKey)
	if n == nil || n.Machine != machine {
		c.mu.Unlock()
		http.Error(w, "node not found", http.StatusNotFound)
		return
	}
	if c.authorized(n) {
		if err := c.update(n, req); err != nil {
			c.mu.Unlock()
			log.Printf("coordinator: map update: %v", err)
			http.Error(w, "could not persist node activity", http.StatusServiceUnavailable)
			return
		}
	} else if !req.Stream {
		c.mu.Unlock()
		http.Error(w, "node keys refused", http.StatusForbidden)
		return
	}
	if !req.Stream {
		c.mu.Unlock()
		return
	}
	wake := make(chan struct{}, 1)
	c.wakes[wake] = true
	n.streams++
	if n.streams == 1 && c.authorized(n) {
		c.notifyNode(n, "online")
	}
	c.changed(n)
	c.mu.Unlock()
	defer func() {
		c.mu.Lock()
		delete(c.wakes, wake)
		if n.streams--; n.streams == 0 {
			if c.authorized(n) && slices.Contains(c.st.Nodes, n) {
				c.notifyNode(n, "offline")
			}
			c.expire(n)
		}
		c.changed(n)
		c.mu.Unlock()
	}()

	sent, host := map[tailcfg.NodeID]uint64{}, ""
	ka := keepAlive + rand.N(jitter)
	tick, tickC := c.clock.NewTicker(ka)
	defer tick.Stop()
	relay, _ := r.Context().Value(relayContextKey{}).(*tailcfg.DERPNode)
	for full := true; ; full = false {
		c.mu.Lock()
		resp, ok := c.delta(n, sent, &host, full, relay)
		c.mu.Unlock()
		if !ok {
			return
		}
		if resp != nil {
			if err := write(w, req.Compress, resp); err != nil {
				return
			}
			if !c.mapProgress(n) {
				return
			}
			tick.Reset(ka)
			if resp.Node != nil && !resp.Node.MachineAuthorized {
				return
			}
		}
		select {
		case <-c.ctx.Done():
			return
		case <-r.Context().Done():
			return
		case <-wake:
		case <-tickC:
			if err := write(w, req.Compress, &tailcfg.MapResponse{KeepAlive: true}); err != nil {
				return
			}
			if !c.mapProgress(n) {
				return
			}
		}
	}
}

// A live poll must survive restart without extending an offline node's lease.
func (c *Coordinator) mapProgress(n *node) bool {
	c.mu.Lock()
	defer c.mu.Unlock()
	if !c.authorized(n) {
		return false
	}
	before := *n
	n.LastMap = c.clock.Now().UTC()
	n.recordActivity(n.LastMap)
	if err := c.save(); err != nil {
		*n = before
		log.Printf("coordinator: map activity: %v", err)
		return false
	}
	c.expire(n)
	return true
}

// update takes what a map request says about n. Peers see a new disco key,
// endpoints or home relay; the rest they do not need.
func (c *Coordinator) update(n *node, req tailcfg.MapRequest) error {
	before := *n
	moved := false
	if !req.DiscoKey.IsZero() && req.DiscoKey != n.Disco {
		n.Disco, moved = req.DiscoKey, true
	}
	if req.Endpoints != nil && !slices.Equal(req.Endpoints, n.Endpoints) {
		n.Endpoints, moved = slices.Clone(req.Endpoints), true
	}
	if hi := req.Hostinfo; hi != nil {
		// A client sends NetInfo only when it changed.
		if hi = hi.Clone(); hi.NetInfo == nil && n.Hostinfo != nil {
			hi.NetInfo = n.Hostinfo.NetInfo
		}
		moved = moved || home(hi) != home(n.Hostinfo)
		n.Hostinfo = hi
	}
	n.capVer = req.Version
	n.LastMap = c.clock.Now().UTC()
	n.recordActivity(c.clock.Now())
	if err := c.save(); err != nil {
		*n = before
		return err
	}
	c.expire(n)
	if moved {
		c.changed(n)
	}
	return nil
}

func (n *node) recordActivity(now time.Time) {
	now = now.UTC()
	if n.JoinedAt.IsZero() {
		n.JoinedAt = now
	}
	if n.LastSeen.Before(now) {
		n.LastSeen = now
	}
}

// Refusal wins over an existing registration, including state written by
// versions that allowed a historical node key to change machines.
func (c *Coordinator) authorized(n *node) bool {
	return n != nil && !n.Removed && !slices.Contains(c.st.RefusedNodes, n.Key) && !slices.Contains(c.st.RefusedMachines, n.Machine)
}

// delta is what n has not seen: itself and every peer it sees whose version
// differs from sent, peers gone from its sight since, and the relay map in
// the full map and when the host moved from host. It is nil when there is
// nothing, and not ok once n itself is gone, or removed since the full map,
// which told it so.
func (c *Coordinator) delta(n *node, sent map[tailcfg.NodeID]uint64, host *string, full bool, relay *tailcfg.DERPNode) (*tailcfg.MapResponse, bool) {
	if !slices.Contains(c.st.Nodes, n) || !c.authorized(n) && !full {
		return nil, false
	}
	r := &tailcfg.MapResponse{}
	live := map[tailcfg.NodeID]bool{n.ID: true}
	if v, ok := sent[n.ID]; full || !ok || v != n.ver {
		sent[n.ID] = n.ver
		r.Node = c.tail(n)
	}
	if full {
		r.Domain = project
		r.PacketFilter = tailcfg.FilterAllowAll
		r.UserProfiles = []tailcfg.UserProfile{{ID: user, LoginName: "iii mesh", DisplayName: "iii mesh"}}
	}
	currentRelay := fmt.Sprintf("%s:%d:%d", c.host, c.relayPort, c.stunPort)
	if relay != nil {
		currentRelay += fmt.Sprintf(":%s:%d:%s", relay.HostName, relay.DERPPort, relay.CertName)
	}
	if full || currentRelay != *host {
		*host = currentRelay
		r.DERPMap = c.relayMap()
		if relay != nil {
			r.DERPMap.Regions[region].Nodes = []*tailcfg.DERPNode{relay}
			if c.stunPort > 0 && c.host != "" {
				r.DERPMap.Regions[region].Nodes = append(r.DERPMap.Regions[region].Nodes, &tailcfg.DERPNode{Name: "hub-stun", RegionID: region, HostName: c.host, STUNOnly: true, STUNPort: c.stunPort})
			}
		}
	}
	for _, p := range c.st.Nodes {
		if p == n || !c.sees(n, p) {
			continue
		}
		live[p.ID] = true
		if v, ok := sent[p.ID]; ok && v == p.ver {
			continue
		}
		sent[p.ID] = p.ver
		if full {
			r.Peers = append(r.Peers, c.tail(p))
		} else {
			r.PeersChanged = append(r.PeersChanged, c.tail(p))
		}
	}
	for id := range sent {
		if !live[id] {
			r.PeersRemoved = append(r.PeersRemoved, id)
			delete(sent, id)
		}
	}
	if !full && r.Node == nil && r.DERPMap == nil && r.PeersChanged == nil && r.PeersRemoved == nil {
		return nil, true
	}
	return r, true
}

// sees is whether a and b are in each other's map, which is the whole access
// rule: WireGuard takes packets only from peers in the map, at both ends. The
// hub sees every node and every node sees only the hub. A removed node sees
// none and is seen by none.
func (c *Coordinator) sees(a, b *node) bool {
	return c.authorized(a) && c.authorized(b) && (a.Name == "hub" || b.Name == "hub")
}

// tail is headscale's TailNode, trimmed to one address and hub services. A
// removed node is no longer authorized.
func (c *Coordinator) tail(n *node) *tailcfg.Node {
	a := netip.PrefixFrom(c.address(n.ID), 128)
	online := n.streams > 0
	var caps tailcfg.NodeCapMap
	if n.Name == "hub" && c.gatewayPort > 0 {
		caps = tailcfg.NodeCapMap{GatewayCapability: {tailcfg.RawMessage(fmt.Sprintf(`{"gateway_port":%d}`, c.gatewayPort))}}
	}
	return &tailcfg.Node{
		CapMap:            caps,
		ID:                n.ID,
		StableID:          tailcfg.StableNodeID(strconv.FormatInt(int64(n.ID), 10)),
		Name:              n.Name,
		User:              user,
		Key:               n.Key,
		Machine:           n.Machine,
		DiscoKey:          n.Disco,
		Addresses:         []netip.Prefix{a},
		AllowedIPs:        []netip.Prefix{a},
		Endpoints:         n.Endpoints,
		HomeDERP:          home(n.Hostinfo),
		Hostinfo:          n.Hostinfo.View(),
		Cap:               n.capVer,
		MachineAuthorized: c.authorized(n),
		Online:            &online,
	}
}

func (c *Coordinator) relayMap() *tailcfg.DERPMap {
	var nodes []*tailcfg.DERPNode
	if c.host != "" {
		nodes = append(nodes, &tailcfg.DERPNode{Name: "hub", RegionID: region, HostName: c.host, DERPPort: c.relayPort, STUNPort: c.stun()})
	}
	return &tailcfg.DERPMap{Regions: map[int]*tailcfg.DERPRegion{region: {RegionID: region, RegionCode: "hub", RegionName: "hub", Nodes: nodes}}}
}

// write is headscale's writeMapResponse: JSON, zstd framed when the client
// asks, behind its little-endian length.
func write(w http.ResponseWriter, compress string, r *tailcfg.MapResponse) error {
	b, err := json.Marshal(r)
	if err != nil {
		return err
	}
	if compress == "zstd" {
		b = zstdframe.AppendEncode(nil, b, zstdframe.FastestCompression)
	}
	if _, err := w.Write(append(binary.LittleEndian.AppendUint32(nil, uint32(len(b))), b...)); err != nil {
		return err
	}
	return http.NewResponseController(w).Flush()
}

func home(hi *tailcfg.Hostinfo) int {
	if hi == nil || hi.NetInfo == nil {
		return 0
	}
	return hi.NetInfo.PreferredDERP
}

// changed gives n a new version, so every poll sends it again.
func (c *Coordinator) changed(n *node) {
	c.ver++
	n.ver = c.ver
	c.wake()
}

func (c *Coordinator) wake() {
	for ch := range c.wakes {
		select {
		case ch <- struct{}{}:
		default:
		}
	}
}

func (c *Coordinator) byKey(k key.NodePublic) *node {
	i := slices.IndexFunc(c.st.Nodes, func(n *node) bool { return n.Key == k })
	if i < 0 {
		return nil
	}
	return c.st.Nodes[i]
}

// byName is the node named name that is not removed.
func (c *Coordinator) byName(name string) *node {
	i := slices.IndexFunc(c.st.Nodes, func(n *node) bool { return n.Name == name && !n.Removed })
	if i < 0 {
		return nil
	}
	return c.st.Nodes[i]
}

func (c *Coordinator) byMachine(k key.MachinePublic) *node {
	i := slices.IndexFunc(c.st.Nodes, func(n *node) bool { return n.Machine == k })
	if i < 0 {
		return nil
	}
	return c.st.Nodes[i]
}
