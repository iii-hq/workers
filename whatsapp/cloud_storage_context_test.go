package main

import (
	"context"
	"encoding/json"
	"errors"
	"testing"
	"time"

	"github.com/iii-hq/workers/whatsapp/internal/databasestore"
)

func TestCloudMessageListCancelsDatabaseRequest(t *testing.T) {
	manager, _ := testSessionManager(t)
	config := manager.Config()
	config.Cloud = cloudTestConfig().Cloud
	if _, err := manager.ApplyConfig(config); err != nil {
		t.Fatal(err)
	}
	inbox, err := manager.CloudInbox(config)
	if err != nil {
		t.Fatal(err)
	}
	if err := inbox.db.Close(); err != nil {
		t.Fatal(err)
	}
	entered := make(chan context.Context, 1)
	release := make(chan struct{})
	inbox.db = databasestore.Open(func(ctx context.Context, _ string, _ any) (json.RawMessage, error) {
		entered <- ctx
		select {
		case <-ctx.Done():
			return nil, ctx.Err()
		case <-release:
			return nil, errors.New("synthetic storage failure")
		}
	}, "whatsapp", "wa_cloud_context_", time.Second)
	registry := NewRegistry(manager)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan error, 1)
	go func() {
		_, err := registry.Invoke(ctx, "whatsapp::cloud::messages::list", map[string]any{"chat_id": "+15551234567"})
		done <- err
	}()
	storageContext := <-entered
	cancel()
	storageCanceled := storageContext.Err() != nil
	close(release)
	if err := <-done; !storageCanceled || !errors.Is(err, context.Canceled) {
		t.Fatalf("Cloud request canceled but storage canceled=%v, returned %v", storageCanceled, err)
	}
}
