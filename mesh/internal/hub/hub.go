package hub

import (
	"context"
	"errors"
	"fmt"
	"os"
	"strings"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	meshclient "github.com/iii-hq/workers/mesh/internal/client"
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
	// Omitted unless tunnel.enabled.
	Tunnel *TunnelStatus `json:"tunnel,omitempty"`
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
	client := iii.New(o.URL, iii.WithOtel(iii.OtelConfig{Enabled: new(bool)}), iii.WithName("mesh"), iii.WithNamespace(namespace))
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
		if cfg.Mode == "client" { // never binds the hub listener nor creates hub state
			if err := h.registerConfiguration(ctx, client, true); err != nil {
				return fmt.Errorf("register configuration: %w", err)
			}
			if err := h.registerUI(client); err != nil {
				return fmt.Errorf("register mesh UI: %w", err)
			}
			// Compose sets III_WORKER_NAME and the SDK applies it over WithName to every
			// later client; the hub-facing clients need their own names.
			os.Unsetenv("III_WORKER_NAME")
			return meshclient.Run(ctx, meshclient.Options{Local: client, Namespace: namespace, Version: o.Version, Fetch: func(ctx context.Context) (string, *meshclient.Config, error) {
				c, err := fetchConfig(ctx, client, h.ConfigID)
				return c.Mode, c.Client, err
			}})
		}
		h, err = Start(ctx, o, cfg, client)
		if err != nil {
			return err
		}
	}
	defer h.Close()
	if err := h.registerNodeEvents(ctx, client); err != nil {
		return fmt.Errorf("register mesh node events: %w", err)
	}
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
	// Runs before h.Close, while the client is still connected.
	defer h.releaseTunnel()
	h.startNodeCalls(ctx)
	h.kickTunnel()
	ticker := time.NewTicker(15 * time.Second)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return nil
		case <-ticker.C:
			h.refreshProxy(ctx)
			h.reconcileTunnel(ctx)
		case <-h.tunnelKick:
			h.reconcileTunnel(ctx)
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
