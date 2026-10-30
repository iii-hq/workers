package hub

import (
	"encoding/json"
	"testing"
)

func TestDefaultConfig(t *testing.T) {
	c := DefaultConfig()
	if c.Listen != "0.0.0.0:49180" || c.GatewayPort != 49135 || c.ProxyURL != "ws://127.0.0.1:49200" || c.StunPort != 0 || c.PublicURL != "" {
		t.Fatalf("defaults: %+v", c)
	}
	if err := c.Validate(); err != nil {
		t.Fatal(err)
	}
}

func TestValidate(t *testing.T) {
	for _, bad := range []string{
		`{"nodes":{"laptop":{"workers":["../ide"]}}}`, `{"nodes":{"laptop":{"workers":["ide@bad version"]}}}`,
		`{"drives":{"work":{"server":"*"}}}`, `{"links":[["laptop","laptop"]]}`,
		`{"gateway_port":0}`, `{"stun_port":65536}`, `{"trusted_proxies":["bad"]}`, `{"public_url":"file:///tmp"}`, `{"listen":"bad"}`, `{"proxy_url":"http://localhost"}`,
	} {
		t.Run(bad, func(t *testing.T) {
			c := DefaultConfig()
			if err := json.Unmarshal([]byte(bad), &c); err != nil {
				t.Fatal(err)
			}
			if c.Validate() == nil {
				t.Fatal("invalid config accepted")
			}
		})
	}
	c := DefaultConfig()
	c.Nodes["laptop"] = NodeConfig{Workers: []string{"ide", "shell@1.0.0"}}
	c.Drives["work"] = DriveConfig{Server: "laptop", Nodes: []string{"*"}}
	if err := c.Validate(); err != nil {
		t.Fatal(err)
	}
}

func TestDriveConfigSeparatesNodeAndDriveNames(t *testing.T) {
	for _, server := range []string{"nas--one", "nas-"} {
		c := DefaultConfig()
		c.Drives["7-project"] = DriveConfig{Server: server, Nodes: []string{"client--one", "client-"}}
		if err := c.Validate(); err != nil {
			t.Fatal("legal node or drive name refused", err)
		}
	}
	for _, drive := range []string{"proj--one", "proj-", "../proj"} {
		c := DefaultConfig()
		c.Drives[drive] = DriveConfig{Server: "nas--one", Nodes: []string{"*"}}
		if c.Validate() == nil {
			t.Fatal("invalid drive name accepted", drive)
		}
	}
}
