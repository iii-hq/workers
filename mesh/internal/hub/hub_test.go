package hub

import (
	"encoding/json"
	"testing"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

func TestStatusReportsVersion(t *testing.T) {
	if got := (&Hub{Version: "0.1.0"}).Status().Version; got != "0.1.0" {
		t.Fatalf("Status version = %q, want %q", got, "0.1.0")
	}
}

func TestStatusSchema(t *testing.T) {
	raw, err := iii.InferSchema[StatusResult]()
	if err != nil {
		t.Fatal(err)
	}
	var schema struct {
		Properties map[string]struct {
			Type       string `json:"type"`
			Properties map[string]struct {
				Type string `json:"type"`
			} `json:"properties"`
		} `json:"properties"`
	}
	if err := json.Unmarshal(raw, &schema); err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"version", "public_url", "prefix", "hub_address", "front_door", "gateway", "proxy"} {
		if got := schema.Properties[name].Type; got != "string" {
			t.Errorf("%s schema type = %q, want string", name, got)
		}
	}
	nodes := schema.Properties["nodes"]
	if nodes.Type != "object" {
		t.Fatalf("nodes schema type = %q, want object", nodes.Type)
	}
	for _, name := range []string{"total", "online", "outdated"} {
		if got := nodes.Properties[name].Type; got != "integer" {
			t.Errorf("nodes.%s schema type = %q, want integer", name, got)
		}
	}
}
