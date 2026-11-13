package hub

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"net"
	"strings"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	meshclient "github.com/iii-hq/workers/mesh/internal/client"
)

// A crashed hub stops being exposed within tunnelLease; a live one renews when
// less than tunnelRenew remains. Variables so tests can lower them.
var (
	tunnelLease = 10 * time.Minute
	tunnelRenew = 5 * time.Minute
)

const tunnelNotResponding = "quick-tunnel is not installed or not responding"

type TunnelStatus struct {
	Enabled  bool   `json:"enabled"`
	TunnelID string `json:"tunnel_id"`
	State    string `json:"state"`
	URL      string `json:"url,omitempty"`
	Error    string `json:"error,omitempty"`
	// "v2" while tunnel connections state the visitor's address
	ProxyProtocol string `json:"proxy_protocol,omitempty"`
}
type TunnelChangeEvent struct {
	TunnelID string `json:"tunnel_id"`
}

// Guarded by tunnelMu. What Status reports lives in the tunnel* fields under mu,
// so a slow quick-tunnel call never blocks mesh::status.
type tunnelState struct {
	leaseID, tunnelID string
	expires           time.Time
	observed          bool
}

func tunnelEnabled(c Config) bool { return c.Tunnel != nil && c.Tunnel.Enabled }
func tunnelIDOf(c Config) string {
	if c.Tunnel != nil && c.Tunnel.TunnelID != "" {
		return c.Tunnel.TunnelID
	}
	return "mesh"
}
func missingPublicURL(c Config) error {
	if tunnelEnabled(c) {
		return errors.New("public_url is not set and the tunnel is not ready")
	}
	return errors.New("public_url is not set")
}

// An explicit public_url wins; otherwise the URL of a ready tunnel. Caller holds mu.
func (h *Hub) publicURLLocked() string {
	if h.config.PublicURL != "" || !tunnelEnabled(h.config) {
		return h.config.PublicURL
	}
	return h.tunnelURL
}
func (h *Hub) publicURL() string { h.mu.RLock(); defer h.mu.RUnlock(); return h.publicURLLocked() }
func (h *Hub) relayURL() string {
	if u := h.publicURL(); u != "" {
		return u
	}
	return h.privateURL
}

func (h *Hub) tunnelStatus(cfg Config) *TunnelStatus {
	if !tunnelEnabled(cfg) {
		return nil
	}
	s := &TunnelStatus{Enabled: true, TunnelID: tunnelIDOf(cfg), State: "overridden"}
	if cfg.PublicURL == "" {
		h.mu.RLock()
		s.State, s.URL, s.Error = h.tunnelPhase, h.tunnelURL, h.tunnelErr
		h.mu.RUnlock()
		if s.State == "" {
			s.State = "starting"
		}
		if h.tunnelProxy.Load() {
			s.ProxyProtocol = "v2"
		}
	}
	return s
}

// wakeTunnel asks the hub's loop for a tunnel pass.
func (h *Hub) wakeTunnel() {
	select {
	case h.tunnelKick <- struct{}{}:
	default:
	}
}

// kickTunnel is wakeTunnel for an event: it may have ended a failure, so the retry starts over.
func (h *Hub) kickTunnel() { h.tunnelRetry.reset(); h.wakeTunnel() }
func (h *Hub) onTunnelChange(e TunnelChangeEvent) ConfigChangeResult {
	if cfg := h.Config(); tunnelEnabled(cfg) && e.TunnelID == tunnelIDOf(cfg) {
		h.kickTunnel()
	}
	return ConfigChangeResult{OK: true}
}

func (h *Hub) tunnelRPC() rpc {
	if h.tunnelClient != nil {
		return h.tunnelClient
	}
	if h.Client != nil {
		return h.Client
	}
	return nil
}
func (h *Hub) tunnelCall(ctx context.Context, client rpc, id string, payload, out any) error {
	b, err := json.Marshal(payload)
	if err != nil {
		return err
	}
	raw, err := client.Call(ctx, iii.CallRequest{FunctionID: id, Namespace: h.Namespace, Payload: b, Timeout: 15 * time.Second})
	if err != nil || out == nil {
		return err
	}
	return json.Unmarshal(raw, out)
}

// Stays inside quick-tunnel's 128-byte id rule whatever the namespace length.
func (h *Hub) tunnelConsumer() string {
	sum := sha256.Sum256([]byte(h.Namespace + "\x00" + h.ConfigID))
	return "mesh:" + hex.EncodeToString(sum[:])[:32]
}

func (h *Hub) publishTunnel(phase, errText, url string) {
	h.mu.Lock()
	before := h.publicURLLocked()
	moved := phase != h.tunnelPhase || errText != h.tunnelErr || url != h.tunnelURL
	h.tunnelPhase, h.tunnelErr, h.tunnelURL = phase, errText, url
	changed := h.publicURLLocked() != before
	h.mu.Unlock()
	if changed {
		h.syncRelay()
	}
	if moved {
		h.statusChanged()
	}
}

// statusChanged tells the observers of mesh::on-status-changed that what the Mesh page reads besides node events moved.
func (h *Hub) statusChanged() {
	h.mu.RLock()
	emit := h.emit
	h.mu.RUnlock()
	if emit == nil {
		return
	}
	if b, err := json.Marshal(h.Status()); err == nil {
		emit(b)
	}
}

// syncRelay points the DERP relay at the current URL. Callers race (tunnel and
// config changes), so the URL is read inside the lock: the last call is current.
func (h *Hub) syncRelay() {
	if h.Coordinator == nil {
		return
	}
	h.relayMu.Lock()
	defer h.relayMu.Unlock()
	h.Coordinator.Relay(h.relayURL())
}

// tunnelFailed reports a failed quick-tunnel call. While the held lease is
// unexpired the tunnel most likely still serves, so its URL stays published.
// It returns whether to try again: quick-tunnel starting ends a missing function or target, and that is an event.
func (h *Hub) tunnelFailed(err error, id, listen string) bool {
	again := !strings.Contains(err.Error(), "unauthorized tunnel_id") && !remoteCode(err, "function_not_found")
	log.Printf("mesh tunnel: %v", err)
	text := tunnelNotResponding
	var remote *iii.InvocationError
	if strings.Contains(err.Error(), "unauthorized tunnel_id") {
		_, port, _ := net.SplitHostPort(listen)
		text = fmt.Sprintf("quick-tunnel has no target %q; add \"%s: http://127.0.0.1:%s\" to its targets", id, id, port)
	} else if errors.As(err, &remote) && remote.Code != "function_not_found" {
		text = "quick-tunnel refused: " + remote.Message
	}
	if h.tunnel.leaseID != "" && time.Now().Before(h.tunnel.expires) {
		h.mu.RLock()
		phase, url := h.tunnelPhase, h.tunnelURL
		h.mu.RUnlock()
		h.publishTunnel(phase, text, url)
		return again
	}
	h.publishTunnel("unavailable", text, "")
	return again
}

// reconcileTunnel holds a quick-tunnel lease while the tunnel is wanted and
// reports its state. Failures are shown in status, never returned.
func (h *Hub) reconcileTunnel(ctx context.Context) {
	h.tunnelMu.Lock()
	defer h.tunnelMu.Unlock()
	// Fail closed: only a status that lists our lease as v2 lets the front door
	// read PROXY headers. Every other way out of here forbids it.
	proxy := false
	defer func() { h.tunnelProxy.Store(proxy) }()
	cfg := h.Config()
	id := tunnelIDOf(cfg)
	want := tunnelEnabled(cfg) && cfg.PublicURL == ""
	retry, failed := false, false // failed: a call failed; retry: and no event ends that
	defer func() {
		if h.tunnelDue != nil {
			h.tunnelDue.Stop()
		}
		held := h.tunnel.leaseID != ""
		if retry || held && (!want || h.tunnel.tunnelID != id) { // or a lease this pass could not release
			h.tunnelRetry.again(h.wakeTunnel)
			return
		}
		h.tunnelRetry.reset()
		if left := time.Until(h.tunnel.expires); held && left > 0 {
			if !failed {
				left -= tunnelRenew // a good pass comes back to renew, a failed one only when the lease ends
			}
			h.tunnelDue = time.AfterFunc(left, h.wakeTunnel)
		}
	}()
	if h.tunnel.leaseID != "" && (!want || h.tunnel.tunnelID != id) {
		h.releaseLease()
	}
	if !want {
		if h.tunnel.leaseID == "" { // a failed release keeps the tunnel up: keep distrusting loopback
			h.tunnelRaw.Store(false)
		}
		h.publishTunnel("", "", "")
		return
	}
	if h.tunnel.leaseID != "" && h.tunnel.tunnelID != id {
		return // the old target's release failed; the retry comes back for it
	}
	client := h.tunnelRPC()
	if client == nil {
		h.publishTunnel("unavailable", tunnelNotResponding, "")
		return
	}
	if !h.tunnel.observed && h.Client != nil {
		h.tunnel.observed = true // before the first acquire: quick-tunnel only reports changes to an observer
		if err := meshclient.Observe(ctx, h.Client, iii.Source{FunctionID: "quick-tunnel::on-changed", Namespace: h.Namespace, Deliver: iii.DeliverInput},
			"mesh::on-tunnel-change", iii.ObserverOptions{ID: "mesh-tunnel", TargetNamespace: h.Namespace}, func() { h.resumed(ctx) }); err != nil {
			log.Printf("mesh tunnel observer: %v", err)
		}
	}
	if h.tunnel.leaseID == "" || time.Until(h.tunnel.expires) < tunnelRenew {
		expires := time.Now().Add(tunnelLease).UTC()
		var lease struct {
			LeaseID string `json:"lease_id"`
		}
		err := h.tunnelCall(ctx, client, "quick-tunnel::acquire", map[string]any{"consumer_id": h.tunnelConsumer(), "tunnel_id": id, "expires_at": expires.Format(time.RFC3339Nano)}, &lease)
		if err == nil && lease.LeaseID == "" {
			err = errors.New("acquire returned no lease")
		}
		if err != nil {
			failed, retry = true, h.tunnelFailed(err, id, cfg.Listen)
			return
		}
		// A lease id we did not hold is a new grant, possibly under another PROXY mode
		// (quick-tunnel restarted): stop reading headers before its tunnel can serve.
		if lease.LeaseID != h.tunnel.leaseID {
			h.tunnelProxy.Store(false)
		}
		h.tunnel = tunnelState{leaseID: lease.LeaseID, tunnelID: id, expires: expires, observed: h.tunnel.observed}
	}
	var snap struct {
		Status    string  `json:"status"`
		PublicURL *string `json:"public_url"`
		Error     *string `json:"error"`
		Provider  string  `json:"provider"`
		Leases    []struct {
			LeaseID       string `json:"lease_id"`
			ProxyProtocol string `json:"proxy_protocol"`
		} `json:"leases"`
	}
	if err := h.tunnelCall(ctx, client, "quick-tunnel::status", map[string]string{"tunnel_id": id}, &snap); err != nil {
		failed, retry = true, h.tunnelFailed(err, id, cfg.Listen)
		return
	}
	listed := false
	for _, l := range snap.Leases {
		if l.LeaseID == h.tunnel.leaseID {
			listed, proxy = true, l.ProxyProtocol == "v2"
		}
	}
	h.tunnelRaw.Store(snap.Provider == "opentunnel")
	if !listed {
		h.tunnel.leaseID, retry = "", true // quick-tunnel dropped it with no event: the retry acquires again
	}
	url, errText := "", ""
	if snap.Error != nil {
		errText = *snap.Error
	}
	// quick-tunnel is another worker: only a clean https URL reaches join codes.
	if snap.Status == "ready" && snap.PublicURL != nil && validURL(*snap.PublicURL, "https") == nil {
		url = *snap.PublicURL
	}
	h.publishTunnel(snap.Status, errText, url)
}

// releaseLease drops the held lease. A failed release keeps it until it expires,
// so the next reconcile retries instead of leaving the tunnel up. Caller holds tunnelMu.
func (h *Hub) releaseLease() {
	if client := h.tunnelRPC(); client != nil && h.tunnel.leaseID != "" {
		ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
		defer cancel()
		if err := h.tunnelCall(ctx, client, "quick-tunnel::release", map[string]string{"lease_id": h.tunnel.leaseID}, nil); err != nil {
			log.Printf("mesh tunnel release: %v", err)
			if time.Now().Before(h.tunnel.expires) {
				return
			}
		}
	}
	h.tunnel = tunnelState{observed: h.tunnel.observed}
}
func (h *Hub) releaseTunnel() {
	h.tunnelMu.Lock()
	defer h.tunnelMu.Unlock()
	h.tunnelProxy.Store(false)
	h.releaseLease()
}
