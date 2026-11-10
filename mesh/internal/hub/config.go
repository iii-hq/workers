package hub

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"github.com/iii-hq/workers/mesh/internal/client"
	"github.com/iii-hq/workers/mesh/internal/drives"
	"io"
	"net"
	"net/netip"
	"net/url"
	"regexp"
	"strconv"
)

type Config struct {
	PublicURL      string                 `json:"public_url"`
	Listen         string                 `json:"listen"`
	TrustedProxies []string               `json:"trusted_proxies"`
	GatewayPort    int                    `json:"gateway_port"`
	ProxyURL       string                 `json:"proxy_url"`
	StunPort       int                    `json:"stun_port"`
	Links          [][2]string            `json:"links"`
	Nodes          map[string]NodeConfig  `json:"nodes"`
	Drives         map[string]DriveConfig `json:"drives"`
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
type NodeConfig struct {
	Workers []string `json:"workers"`
	Allow   []string `json:"allow"`
}
type DriveConfig struct {
	Server string   `json:"server"`
	Nodes  []string `json:"nodes"`
}

func DefaultConfig() Config {
	return Config{Listen: "0.0.0.0:49180", GatewayPort: 49135, ProxyURL: "ws://127.0.0.1:49200", TrustedProxies: []string{}, Links: [][2]string{}, Nodes: map[string]NodeConfig{}, Drives: map[string]DriveConfig{}}
}

var workerPattern = regexp.MustCompile(`^[a-z0-9-]+(@\S+)?$`)
var namePattern = regexp.MustCompile(`^[a-z][a-z0-9-]{0,30}$`)
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
	for name, n := range c.Nodes {
		if !namePattern.MatchString(name) {
			return fmt.Errorf("invalid node name %q", name)
		}
		for _, w := range n.Workers {
			if !workerPattern.MatchString(w) {
				return fmt.Errorf("invalid worker %q", w)
			}
		}
	}
	for name, d := range c.Drives {
		if !drives.ValidDrive(name) || !drives.ValidNode(d.Server) {
			return fmt.Errorf("invalid drive %q or server", name)
		}
		for _, n := range d.Nodes {
			if n != "*" && !namePattern.MatchString(n) {
				return fmt.Errorf("invalid drive node %q", n)
			}
		}
	}
	for _, l := range c.Links {
		if l[0] == l[1] || !namePattern.MatchString(l[0]) || !namePattern.MatchString(l[1]) {
			return errors.New("invalid link")
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
