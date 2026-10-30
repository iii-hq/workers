package hub

import (
	"context"
	"fmt"
	"os"
	"strings"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

type Options struct {
	URL     string
	Version string
}

type StatusResult struct {
	Version    string     `json:"version"`
	PublicURL  string     `json:"public_url"`
	Prefix     string     `json:"prefix"`
	HubAddress string     `json:"hub_address"`
	FrontDoor  string     `json:"front_door"`
	Gateway    string     `json:"gateway"`
	Proxy      string     `json:"proxy"`
	Nodes      NodeCounts `json:"nodes"`
}

type NodeCounts struct {
	Total    int `json:"total"`
	Online   int `json:"online"`
	Outdated int `json:"outdated"`
}

func Status(version string) StatusResult {
	return StatusResult{Version: version}
}

func Run(ctx context.Context, o Options) error {
	namespace := strings.TrimSpace(os.Getenv("III_NAMESPACE"))
	if namespace == "" {
		namespace = "default"
	}
	client := iii.New(o.URL, iii.WithName("mesh"), iii.WithNamespace(namespace))
	defer client.Close()

	connectCtx, cancel := context.WithTimeout(ctx, 30*time.Second)
	err := client.Connect(connectCtx)
	cancel()
	if err != nil {
		return fmt.Errorf("connect to iii engine: %w", err)
	}
	if err := iii.RegisterFunctionTyped(client, "mesh::status", func(context.Context, struct{}) (StatusResult, error) {
		return Status(o.Version), nil
	}); err != nil {
		return fmt.Errorf("register mesh::status: %w", err)
	}
	<-ctx.Done()
	return nil
}
