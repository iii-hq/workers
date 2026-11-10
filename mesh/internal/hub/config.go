package hub

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"github.com/iii-hq/workers/mesh/internal/client"
	"io"
	"net"
	"net/netip"
	"net/url"
	"regexp"
	"strconv"
)

type Config struct {
	PublicURL      string   `json:"public_url"`
	Listen         string   `json:"listen"`
	TrustedProxies []string `json:"trusted_proxies"`
	GatewayPort    int      `json:"gateway_port"`
	ProxyURL       string   `json:"proxy_url"`
	StunPort       int      `json:"stun_port"`
	// Retired on 2026-11-10; accepted and ignored. Entries stored before then hold
	// these keys, and the engine checks a stored value against the schema reflected
	// from this struct, which refuses unknown keys.
	Links  json.RawMessage `json:"links,omitempty" jsonschema:"description=Retired on 2026-11-10; ignored."`
	Nodes  json.RawMessage `json:"nodes,omitempty" jsonschema:"description=Retired on 2026-11-10; ignored."`
	Drives json.RawMessage `json:"drives,omitempty" jsonschema:"description=Retired on 2026-11-10; ignored."`
	// Mode and Client are omitempty so stored values without them still decode
	// and the schema sent to configuration::ensure gains no required field.
	Mode   string         `json:"mode,omitempty" jsonschema:"enum=hub,enum=client"`
	Client *client.Config `json:"client,omitempty"`
	// Hub mode only. Without public_url the hub leases a quick-tunnel target and
	// uses its URL as the public URL.
	Tunnel *TunnelConfig `json:"tunnel,omitempty"`
}
type TunnelConfig struct {
	Enabled  bool   `json:"enabled"`
	TunnelID string `json:"tunnel_id,omitempty"` // default "mesh"
}

func DefaultConfig() Config {
	return Config{Listen: "0.0.0.0:49180", GatewayPort: 49135, ProxyURL: "ws://127.0.0.1:49200", TrustedProxies: []string{}}
}

var tunnelIDPattern = regexp.MustCompile(`^[A-Za-z0-9_:.-]{1,128}$`)

func (c Config) Validate() error {
	switch c.Mode {
	case "", "hub", "client":
	default:
		return fmt.Errorf("invalid mode %q", c.Mode)
	}
	// Checked in every mode: a running client applies client.share whatever the mode says.
	if cc := c.Client; cc != nil {
		if cc.HubURL != "" {
			if err := validURL(cc.HubURL, "http", "https"); err != nil {
				return fmt.Errorf("client.hub_url: %w", err)
			}
		}
		if err := cc.Validate(); err != nil {
			return fmt.Errorf("client: %w", err)
		}
	}
	if t := c.Tunnel; t != nil && t.TunnelID != "" && !tunnelIDPattern.MatchString(t.TunnelID) {
		return fmt.Errorf("invalid tunnel.tunnel_id %q", t.TunnelID)
	}
	if c.PublicURL != "" {
		if err := validURL(c.PublicURL, "http", "https"); err != nil {
			return fmt.Errorf("public_url: %w", err)
		}
	}
	if err := validURL(c.ProxyURL, "ws", "wss"); err != nil {
		return fmt.Errorf("proxy_url: %w", err)
	}
	_, port, err := net.SplitHostPort(c.Listen)
	if err != nil {
		return fmt.Errorf("listen: %w", err)
	}
	p, err := strconv.Atoi(port)
	if err != nil || p < 0 || p > 65535 {
		return errors.New("invalid listen port")
	}
	if c.GatewayPort < 1 || c.GatewayPort > 65535 {
		return errors.New("invalid gateway_port")
	}
	if c.StunPort < 0 || c.StunPort > 65535 {
		return errors.New("invalid stun_port")
	}
	if c.StunPort > 0 && c.PublicURL == "" {
		return errors.New("public_url is required for STUN")
	}
	for _, s := range c.TrustedProxies {
		if _, err := netip.ParsePrefix(s); err != nil {
			return fmt.Errorf("trusted_proxies: %w", err)
		}
	}
	return nil
}
func validURL(raw string, schemes ...string) error {
	u, err := url.Parse(raw)
	if err != nil {
		return err
	}
	allowed := false
	for _, s := range schemes {
		allowed = allowed || u.Scheme == s
	}
	if !allowed || u.Hostname() == "" || u.User != nil || u.RawQuery != "" || u.ForceQuery || u.Fragment != "" {
		return errors.New("expected an absolute URL without credentials, query or fragment")
	}
	if u.Port() != "" {
		p, e := strconv.Atoi(u.Port())
		if e != nil || p < 1 || p > 65535 {
			return errors.New("invalid URL port")
		}
	}
	return nil
}
func parseConfig(raw json.RawMessage) (Config, error) {
	c := DefaultConfig()
	if len(raw) == 0 || bytes.Equal(bytes.TrimSpace(raw), []byte("null")) {
		return c, errors.New("missing configuration value")
	}
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.DisallowUnknownFields()
	if err := dec.Decode(&c); err != nil {
		return c, err
	}
	if err := dec.Decode(new(any)); err != io.EOF {
		return c, errors.New("trailing configuration data")
	}
	return c, c.Validate()
}
func cloneConfig(c Config) Config {
	b, _ := json.Marshal(c)
	var out Config
	_ = json.Unmarshal(b, &out)
	return out
}
