package hub

import (
	"context"
	"errors"
	"fmt"
	"os"
	"strings"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

type Options struct {
	URL              string
	Version          string
	DataDir          string
	LocalConfig      bool
	CollectInterface bool
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

func Run(ctx context.Context, o Options) (err error) {
	defer func() {
		if ctx.Err() != nil && errors.Is(err, ctx.Err()) {
			err = nil
		}
	}()
	namespace := strings.TrimSpace(os.Getenv("III_NAMESPACE"))
	if namespace == "" {
		namespace = "default"
	}
	client := iii.New(o.URL, iii.WithName("mesh"), iii.WithNamespace(namespace))
	defer client.Close()

	connectCtx, cancel := context.WithTimeout(ctx, 30*time.Second)
	err = client.Connect(connectCtx)
	cancel()
	if err != nil {
		return fmt.Errorf("connect to iii engine: %w", err)
	}

	h := &Hub{Version: o.Version, Namespace: namespace, ConfigID: configID(), Client: client, config: DefaultConfig(), Sessions: &Sessions{}}
	local := o.LocalConfig || o.CollectInterface
	if !local {
		cfg, err := ensureConfig(ctx, client, h.ConfigID)
		if err != nil {
			return err
		}
		h, err = Start(ctx, o, cfg, client)
		if err != nil {
			return err
		}
	}
	defer h.Close()
	if err := h.registerFunctions(client); err != nil {
		return fmt.Errorf("register mesh functions: %w", err)
	}
	if err := h.registerProxyFunctions(ctx, o.URL); err != nil {
		return fmt.Errorf("register proxy functions: %w", err)
	}
	if err := h.registerConfiguration(ctx, client, local); err != nil {
		return fmt.Errorf("register configuration: %w", err)
	}

	if err := h.registerUI(client); err != nil {
		return fmt.Errorf("register mesh UI: %w", err)
	}

	if local {
		<-ctx.Done()
		return nil
	}
	h.startNodeCalls(ctx)
	ticker := time.NewTicker(15 * time.Second)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return nil
		case <-ticker.C:
			h.refreshProxy(ctx)
		}
	}
}

func namespace() string {
	if s := strings.TrimSpace(os.Getenv("III_NAMESPACE")); s != "" {
		return s
	}
	return "default"
}
func configID() string {
	if s := strings.TrimSpace(os.Getenv("III_CONFIG_NAME")); s != "" {
		return s
	}
	return "mesh"
}
