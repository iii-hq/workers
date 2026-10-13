package main

import (
	"context"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

// Collection uses the production registrations with an ephemeral manager. It
// never opens account storage, reads tunnel state, or invokes another worker.
func newInterfaceManager(config Config) (*Manager, error) {
	if err := config.Validate(); err != nil {
		return nil, err
	}
	return &Manager{
		interfaceOnly:   true,
		config:          config,
		sessions:        map[string]*Session{},
		cloud:           map[string]*Inbox{},
		EventQueue:      make(chan Event),
		CloudEventQueue: make(chan CloudEvent),
	}, nil
}

func registerWorkerInterface(ctx context.Context, client *iii.Client, manager *Manager, engineURL string, local bool) error {
	if err := registerEvents(ctx, client, manager); err != nil {
		return err
	}
	if err := registerCloudEvents(ctx, client, manager); err != nil {
		return err
	}
	if err := registerCloudWebhooks(client, manager, engineURL); err != nil {
		return err
	}
	if err := registerCloudFlowEndpoint(client, manager, engineURL); err != nil {
		return err
	}
	if err := registerDevelopmentTunnel(ctx, client, manager); err != nil {
		return err
	}
	operationManager := manager
	if manager.interfaceOnly {
		operationManager = nil
	}
	if err := NewRegistry(operationManager).Install(client); err != nil {
		return err
	}
	if err := registerUI(client); err != nil {
		return err
	}
	return registerConfiguration(ctx, client, manager, local)
}
