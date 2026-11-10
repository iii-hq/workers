package hub

import (
	"encoding/json"
	"strings"
	"testing"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
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

func TestClientModeConfig(t *testing.T) {
	old := `{"public_url":"","listen":"0.0.0.0:49180","trusted_proxies":[],"gateway_port":49135,"proxy_url":"ws://127.0.0.1:49200","stun_port":0,"links":[],"nodes":{},"drives":{}}`
	c, err := parseConfig(json.RawMessage(old))
	if err != nil || c.Mode != "" || c.Client != nil {
		t.Fatalf("stored value without mode: %+v %v", c, err)
	}
	if _, err = parseConfig(json.RawMessage(`{"mode":"client","client":{"hub_url":"http://192.0.2.1:49180","join":"secret://MESH_JOIN","name":"harness-mbp","share":["mesh-probe"]}}`)); err != nil {
		t.Fatal(err)
	}
	if _, err = parseConfig(json.RawMessage(`{"mode":"client"}`)); err != nil {
		t.Fatal("client mode without settings must wait for a join, not fail:", err)
	}
	for _, bad := range []string{
		`{"mode":"bogus"}`,
		`{"mode":"client","client":{"share":["secrets"]}}`,
		`{"mode":"client","client":{"join":"ABCD-EFGH"}}`,
		`{"mode":"client","client":{"hub_url":"ftp://hub"}}`,
		`{"mode":"client","client":{"unknown":true}}`,
		`{"mode":"hub","client":{"share":["secrets"]}}`,
		`{"client":{"join":"ABCD-EFGH"}}`,
	} {
		if _, err := parseConfig(json.RawMessage(bad)); err == nil {
			t.Fatalf("accepted %s", bad)
		}
	}
}

func TestSchemaHasNoRequiredClientFields(t *testing.T) {
	raw, err := iii.InferSchema[Config]()
	if err != nil {
		t.Fatal(err)
	}
	var schema struct {
		Required   []string `json:"required"`
		Properties map[string]struct {
			Enum     []string `json:"enum"`
			Required []string `json:"required"`
		} `json:"properties"`
	}
	if err = json.Unmarshal(raw, &schema); err != nil {
		t.Fatal(err)
	}
	for _, name := range schema.Required {
		if name == "mode" || name == "client" {
			t.Fatalf("%s is required: %s", name, raw)
		}
	}
	if len(schema.Properties["client"].Required) != 0 || len(schema.Properties["mode"].Enum) != 2 {
		t.Fatalf("schema %s", raw)
	}
}

func TestTunnelConfig(t *testing.T) {
	old := `{"public_url":"","listen":"0.0.0.0:49180","trusted_proxies":[],"gateway_port":49135,"proxy_url":"ws://127.0.0.1:49200","stun_port":0,"links":[],"nodes":{},"drives":{}}`
	c, err := parseConfig(json.RawMessage(old))
	if err != nil || c.Tunnel != nil {
		t.Fatalf("stored value without tunnel: %+v %v", c, err)
	}
	if b, _ := json.Marshal(c); strings.Contains(string(b), `"tunnel"`) {
		t.Fatalf("absent tunnel serialized: %s", b)
	}
	for _, good := range []string{`{"tunnel":{"enabled":true}}`, `{"tunnel":{"enabled":false,"tunnel_id":"mesh.hub:1_a-b"}}`} {
		if _, err := parseConfig(json.RawMessage(good)); err != nil {
			t.Fatal(good, err)
		}
	}
	for _, bad := range []string{`{"tunnel":{"enabled":true,"tunnel_id":"bad id"}}`, `{"tunnel":{"enabled":true,"tunnel_id":"` + strings.Repeat("a", 129) + `"}}`, `{"tunnel":{"bogus":true}}`} {
		if _, err := parseConfig(json.RawMessage(bad)); err == nil {
			t.Fatalf("accepted %s", bad)
		}
	}
	c.Tunnel = &TunnelConfig{Enabled: true, TunnelID: "mesh"}
	d := cloneConfig(c)
	d.Tunnel.Enabled = false
	if !c.Tunnel.Enabled {
		t.Fatal("cloneConfig aliases tunnel")
	}
}
