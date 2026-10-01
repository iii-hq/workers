package main

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"testing"
	"time"

	"go.mau.fi/whatsmeow"
	waLog "go.mau.fi/whatsmeow/util/log"
)

func testSessionManager(t *testing.T) (*Manager, *Session) {
	t.Helper()
	cfg := DefaultConfig()
	cfg.DataDir = filepath.Join(t.TempDir(), "sessions")
	m, err := NewManager(cfg)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		if err := m.Shutdown(ctx); err != nil {
			t.Errorf("cleanup: %v", err)
		}
	})
	if _, err = m.Create("test"); err != nil {
		t.Fatal(err)
	}
	s, err := m.session("test")
	if err != nil {
		t.Fatal(err)
	}
	return m, s
}

// Build a real, offline library client. This helper never calls Connect.
func testOfflineNativeClient(t *testing.T, s *Session) *whatsmeow.Client {
	t.Helper()
	container, err := openDevice(context.Background(), s.dir)
	if err != nil {
		t.Fatal(err)
	}
	device, err := container.GetFirstDevice(context.Background())
	if err != nil {
		container.Close()
		t.Fatal(err)
	}
	client := whatsmeow.NewClient(device, waLog.Noop)
	s.mu.Lock()
	s.client, s.container, s.acceptEvents, s.status = client, container, true, "starting"
	s.mu.Unlock()
	return client
}

func TestDeviceSQLiteEscapesFilenameAndRejectsSymlink(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "data?mode=memory#fragment")
	if err := privateInboxDirectory(dir); err != nil {
		t.Fatal(err)
	}
	container, err := openDevice(context.Background(), dir)
	if err != nil {
		t.Fatal(err)
	}
	if _, err = container.GetFirstDevice(context.Background()); err != nil {
		t.Fatal(err)
	}
	if err = container.Close(); err != nil {
		t.Fatal(err)
	}
	info, err := os.Stat(filepath.Join(dir, "device.db"))
	if err != nil || info.Size() == 0 || info.Mode().Perm() != 0600 {
		t.Fatalf("device DB was not written to exact private path: %v, %v", info, err)
	}
	container, err = openDevice(context.Background(), dir)
	if err != nil {
		t.Fatal(err)
	}
	container.Close()
	unsafe := filepath.Join(t.TempDir(), "unsafe")
	if err = os.Mkdir(unsafe, 0700); err != nil {
		t.Fatal(err)
	}
	if err = os.Symlink(filepath.Join(dir, "device.db"), filepath.Join(unsafe, "device.db")); err != nil {
		t.Fatal(err)
	}
	if _, err = openDevice(context.Background(), unsafe); err == nil {
		t.Fatal("symlink device DB accepted")
	}
}

func TestDeletedSessionCannotAcquireNewIncarnation(t *testing.T) {
	m, old := testSessionManager(t)
	if _, err := m.Delete(context.Background(), "test"); err != nil {
		t.Fatal(err)
	}
	if _, err := m.Create("test"); err != nil {
		t.Fatal(err)
	}
	current, err := m.session("test")
	if err != nil || current == old {
		t.Fatal("session was not recreated")
	}
	if unlock, err := m.lockTransition(context.Background(), old); err == nil {
		unlock()
		t.Fatal("stale session acquired lifecycle transition")
	}
	if _, err := current.inbox.ListChats("", "", 10); err != nil {
		t.Fatalf("new incarnation damaged: %v", err)
	}
	if _, err := os.Stat(filepath.Join(current.dir, "metadata.json")); err != nil {
		t.Fatal(err)
	}
}

func TestStopDeadlineKeepsResourcesUntilConnectDrains(t *testing.T) {
	m, s := testSessionManager(t)
	client := testOfflineNativeClient(t, s)
	container := s.container
	done := make(chan struct{})
	s.connectDone = done
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Millisecond)
	defer cancel()
	if _, err := m.Stop(ctx, "test", false); !errors.Is(err, context.DeadlineExceeded) {
		close(done)
		t.Fatalf("stop: %v", err)
	}
	if s.client != client || s.container != container || s.connectDone != done || s.acceptEvents {
		close(done)
		t.Fatal("timed-out stop lost cleanup handles or still accepts events")
	}
	if _, err := m.Client("test"); err == nil {
		close(done)
		t.Fatal("stopping client remained callable")
	}
	close(done)
	if _, err := m.Stop(context.Background(), "test", false); err != nil {
		t.Fatal(err)
	}
	if s.client != nil || s.container != nil || s.connectDone != nil || s.status != "stopped" {
		t.Fatal("retry did not finish cleanup")
	}
}

func TestLifecycleWaitHonorsDeadlineWithoutDeferredMutation(t *testing.T) {
	m, s := testSessionManager(t)
	s.operations.RLock()
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Millisecond)
	defer cancel()
	_, err := m.Delete(ctx, "test")
	s.operations.RUnlock()
	if !errors.Is(err, context.DeadlineExceeded) {
		t.Fatalf("delete: %v", err)
	}
	if _, err = m.session("test"); err != nil {
		t.Fatal("cancelled request deleted session")
	}
	if _, err = m.Stop(context.Background(), "test", false); err != nil {
		t.Fatal("cancelled request retained lock", err)
	}
}

func TestStopUsesConfiguredShutdownTimeout(t *testing.T) {
	m, s := testSessionManager(t)
	cfg := m.Config()
	cfg.ShutdownTimeoutSecs = 1
	if _, err := m.ApplyConfig(cfg); err != nil {
		t.Fatal(err)
	}
	s.operations.RLock()
	started := time.Now()
	_, err := m.Stop(context.Background(), "test", false)
	s.operations.RUnlock()
	if !errors.Is(err, context.DeadlineExceeded) || time.Since(started) > 3*time.Second {
		t.Fatalf("configured stop timeout ignored: %v after %s", err, time.Since(started))
	}
}

func TestShutdownDeadlineRetainsWorkerLockUntilDrained(t *testing.T) {
	m, s := testSessionManager(t)
	cfg := m.Config()
	s.operations.RLock()
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Millisecond)
	defer cancel()
	err := m.Shutdown(ctx)
	if !errors.Is(err, context.DeadlineExceeded) {
		s.operations.RUnlock()
		t.Fatalf("shutdown: %v", err)
	}
	if second, err := NewManager(cfg); err == nil {
		s.operations.RUnlock()
		second.Shutdown(context.Background())
		t.Fatal("worker lock released before active operations drained")
	}
	if _, err := m.Create("new"); err == nil {
		s.operations.RUnlock()
		t.Fatal("new session accepted during shutdown")
	}
	s.operations.RUnlock()
	ctx2, cancel2 := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel2()
	if err = m.Shutdown(ctx2); err != nil {
		t.Fatal(err)
	}
	reopened, err := NewManager(cfg)
	if err != nil {
		t.Fatal("lock retained after complete shutdown", err)
	}
	defer reopened.Shutdown(context.Background())
}

func TestSessionStartupRetainsAccountsAboveLoweredLimit(t *testing.T) {
	m, _ := testSessionManager(t)
	if _, err := m.Create("second"); err != nil {
		t.Fatal(err)
	}
	cfg := m.Config()
	cfg.MaxSessions = 1
	if _, err := m.ApplyConfig(cfg); err != nil {
		t.Fatal(err)
	}
	if _, err := m.Create("third"); err == nil {
		t.Fatal("lowered session limit ignored")
	}
	if err := m.Shutdown(context.Background()); err != nil {
		t.Fatal(err)
	}
	reopened, err := NewManager(cfg)
	if err != nil {
		t.Fatal("existing accounts prevented startup", err)
	}
	defer reopened.Shutdown(context.Background())
	if len(reopened.sessions) != 2 {
		t.Fatal("existing account discarded")
	}
}

func TestCloudInboxRejectsDisabledAndObsoleteConfig(t *testing.T) {
	m, _ := testSessionManager(t)
	cfg := m.Config()
	if _, err := m.CloudInbox(cfg); err == nil {
		t.Fatal("disabled Cloud archive opened")
	}
	cfg.Cloud.Enabled, cfg.Cloud.PhoneNumberID = true, "123456789"
	cfg.Cloud.BusinessAccountID, cfg.Cloud.AccessToken = "987654321", "unit-test-token"
	if _, err := m.ApplyConfig(cfg); err != nil {
		t.Fatal(err)
	}
	if _, err := m.CloudInbox(cfg); err != nil {
		t.Fatal(err)
	}
	old := cfg
	cfg.Cloud.PhoneNumberID = "123456780"
	if _, err := m.ApplyConfig(cfg); err != nil {
		t.Fatal(err)
	}
	if _, err := m.CloudInbox(old); err == nil {
		t.Fatal("obsolete Cloud archive opened")
	}
}
