package node

import (
	"context"
	"fmt"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

// Run owns its SDK connection, reconnecting it when authenticated gateway
// metadata changes. The caller stops borrowed controllers and mesh on return.
func (n *Node) Run(ctx context.Context) error {
	var client *iii.Client
	var sessionCancel context.CancelFunc
	var connected <-chan error
	var activeURL, activeName string
	closeSession := func() {
		if sessionCancel != nil {
			sessionCancel()
			sessionCancel = nil
		}
		if client != nil {
			client.Close()
			client = nil
		}
		connected = nil
	}
	defer closeSession()
	refresh := func() error {
		name, named := n.mesh.SelfName()
		port, advertised := n.mesh.GatewayPort()
		url := ""
		if named && advertised {
			url = GatewayURL(port)
		}
		if url == activeURL && name == activeName {
			return nil
		}
		closeSession()
		activeURL, activeName = url, name
		n.applyMu.Lock()
		if n.namespace != "" {
			err := n.workers.SetRuntime(ctx, Runtime{Node: name, Namespace: n.namespace, URL: url})
			if err != nil {
				n.applyMu.Unlock()
				return fmt.Errorf("update worker runtime: %w", err)
			}
		}
		n.applyMu.Unlock()
		if url == "" {
			return nil
		}
		client = iii.New(url, iii.WithName("mesh-node"), iii.WithNamespace("default"))
		if err := n.Register(client); err != nil {
			return err
		}
		sessionCtx, cancel := context.WithCancel(ctx)
		sessionCancel = cancel
		result := make(chan error, 1)
		connected = result
		current := client
		go func() { result <- current.Connect(sessionCtx) }()
		return nil
	}
	changes := n.mesh.GatewayChanges()
	ticker := time.NewTicker(time.Second)
	defer ticker.Stop()
	if err := refresh(); err != nil {
		return err
	}
	for {
		select {
		case <-ctx.Done():
			return nil
		case <-n.restart:
			return ErrRestartRequired
		case <-n.mesh.Removed():
			return ErrRemoved
		case err := <-connected:
			connected = nil
			if err != nil && ctx.Err() == nil {
				return fmt.Errorf("connect node service: %w", err)
			}
		case _, ok := <-changes:
			if !ok {
				return ErrRemoved
			}
			if err := refresh(); err != nil {
				return err
			}
		case <-ticker.C:
			if err := refresh(); err != nil {
				return err
			}
		}
	}
}
