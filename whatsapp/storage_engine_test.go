package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/whatsapp/internal/databasestore"
	"go.mau.fi/whatsmeow/proto/waAdv"
	"go.mau.fi/whatsmeow/store/sqlstore"
	"go.mau.fi/whatsmeow/types"
)

func isolatedDatabaseStorage(t *testing.T) (*databaseStorage, Config, string) {
	t.Helper()
	engine, database := os.Getenv("III_BINARY"), os.Getenv("DATABASE_BINARY")
	if engine == "" || database == "" {
		t.Skip("set III_BINARY and DATABASE_BINARY for real database worker integration")
	}
	dir := t.TempDir()
	socket, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	port := socket.Addr().(*net.TCPAddr).Port
	socket.Close()
	engineURL := fmt.Sprintf("ws://127.0.0.1:%d", port)
	engineConfig := filepath.Join(dir, "engine.yaml")
	if err = os.WriteFile(engineConfig, []byte(fmt.Sprintf("workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: %d\n  - name: configuration\n    config:\n      adapter:\n        name: fs\n        config:\n          directory: ./configuration\n", port)), 0600); err != nil {
		t.Fatal(err)
	}
	smokeProcess(t, dir, "engine", engine, []string{"--config", engineConfig, "--no-update-check"}, "III_NAMESPACE=default")
	smokeWait(t, "engine", func() bool {
		_, err := smokeValue(engine, engineURL, "default", "engine::workers::list", map[string]any{})
		return err == nil
	})
	seed := filepath.Join(dir, "database.yaml")
	if err = os.WriteFile(seed, []byte("databases:\n  whatsapp:\n    url: sqlite:"+filepath.Join(dir, "whatsapp.db")+"\n    pool:\n      max: 1\nhistory_max_entries: 0\nhistory_max_bytes: 0\n"), 0600); err != nil {
		t.Fatal(err)
	}
	smokeProcess(t, dir, "database", database, []string{"--url", engineURL, "--config", seed}, "III_NAMESPACE="+smokeNamespace, "III_CONFIG_NAME=storage-test-database", "III_DISABLE_TRACE_PAYLOADS=1")
	smokeWait(t, "database worker", func() bool {
		_, err := smokeValue(engine, engineURL, smokeNamespace, "database::listDatabases", map[string]any{})
		return err == nil
	})
	client := iii.New(engineURL, iii.WithName("storage-test"))
	if err = client.Connect(context.Background()); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { client.Close() })
	call := databaseCall(client, smokeNamespace, 10*time.Second)
	// Synthetic fixtures only. Expose SQL errors to this isolated test without
	// weakening the production adapter's sanitized errors.
	checked := func(ctx context.Context, id string, value any) (json.RawMessage, error) {
		raw, e := call(ctx, id, value)
		if e != nil {
			t.Logf("synthetic storage call %s failed: %v", id, e)
		}
		return raw, e
	}
	if err = databasestore.Validate(context.Background(), checked, "whatsapp", 10*time.Second); err != nil {
		t.Fatal(err)
	}
	config := DefaultConfig()
	config.DataDir = filepath.Join(dir, "accounts")
	config.Cloud = cloudTestConfig().Cloud
	storage := &databaseStorage{call: checked, db: "whatsapp", root: config.DataDir, namespace: smokeNamespace, configID: "storage-test-whatsapp", timeout: 10 * time.Second}
	return storage, config, engineURL
}
func saveSyntheticDevice(t *testing.T, c *sqlstore.Container, user string) []byte {
	t.Helper()
	ctx := context.Background()
	device := c.NewDevice()
	id := types.NewJID(user, types.DefaultUserServer)
	device.ID = &id
	device.Account = &waAdv.ADVSignedDeviceIdentity{Details: []byte{1}, AccountSignature: bytes.Repeat([]byte{2}, 64), AccountSignatureKey: bytes.Repeat([]byte{3}, 32), DeviceSignature: bytes.Repeat([]byte{4}, 64)}
	if err := device.Save(ctx); err != nil {
		t.Fatal(err)
	}
	if err := device.Sessions.PutSession(ctx, "synthetic-peer", []byte{0, 255, 42}); err != nil {
		t.Fatal(err)
	}
	return append([]byte(nil), device.NoiseKey.Priv[:]...)
}
func TestDatabaseWorkerPersistence(t *testing.T) {
	storage, config, _ := isolatedDatabaseStorage(t)
	ctx := context.Background()
	m, err := NewManager(config, storage)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { m.Shutdown(ctx) })
	for _, id := range []string{"one", "two"} {
		if _, err = m.Create(id); err != nil {
			t.Fatal(err)
		}
	}
	one, _ := m.Inbox("one")
	two, _ := m.Inbox("two")
	msg := MessageView{ChatID: "15551234567@s.whatsapp.net", SenderID: "15551234567@s.whatsapp.net", MessageID: "native-test", TimestampMS: 1700000000000, Kind: "text", Text: "Synthetic archive", Source: "live"}
	if err = one.Record([]MessageView{msg}); err != nil {
		t.Fatal(err)
	}
	page, err := two.ListChats("", "", 10)
	if err != nil || len(page.Chats) != 0 {
		t.Fatal("cross-session archive leak", err)
	}
	oneSession, _ := m.session("one")
	twoSession, _ := m.session("two")
	c, err := openStoredDevice(ctx, oneSession.dir, storage)
	if err != nil {
		t.Fatal(err)
	}
	key := saveSyntheticDevice(t, c, "15550000001")
	c.Close()
	c, err = openStoredDevice(ctx, twoSession.dir, storage)
	if err != nil {
		t.Fatal(err)
	}
	other, err := c.GetFirstDevice(ctx)
	if err != nil || other.ID != nil {
		t.Fatal("cross-session device leak", err)
	}
	c.Close()
	c, err = openStoredDevice(ctx, oneSession.dir, storage)
	if err != nil {
		t.Fatal(err)
	}
	d, err := c.GetFirstDevice(ctx)
	if err != nil || d.ID == nil || !bytes.Equal(d.NoiseKey.Priv[:], key) {
		t.Fatal("native keys did not survive reopening", err)
	}
	blob, err := d.Sessions.GetSession(ctx, "synthetic-peer")
	if err != nil || !bytes.Equal(blob, []byte{0, 255, 42}) {
		t.Fatal("native binary session corrupted", err)
	}
	c.Close()
	cloud, err := m.CloudInbox(config)
	if err != nil {
		t.Fatal(err)
	}
	batch := cloudTestParse(t, cloudTestPayload(cloudTestChange(cloudTestMessage("storage-cloud"))))
	events, err := cloud.RecordCloudDelivery(batch.messages, batch.receipts, batch.events)
	if err != nil || len(events) != 1 {
		t.Fatal("Cloud archive write", err)
	}
	events, err = cloud.RecordCloudDelivery(batch.messages, batch.receipts, batch.events)
	if err != nil || len(events) != 0 {
		t.Fatal("Cloud deduplication", err)
	}
	if err = m.Shutdown(ctx); err != nil {
		t.Fatal(err)
	}
	m, err = NewManager(config, storage)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { m.Shutdown(ctx) })
	one, _ = m.Inbox("one")
	messages, err := one.ListMessages(msg.ChatID, "", 10)
	if err != nil || len(messages.Messages) != 1 {
		t.Fatal("native archive lost on restart", err)
	}
	cloud, err = m.CloudInbox(config)
	if err != nil {
		t.Fatal(err)
	}
	events, err = cloud.RecordCloudDelivery(batch.messages, batch.receipts, batch.events)
	if err != nil || len(events) != 0 {
		t.Fatal("Cloud deduplication lost on restart", err)
	}
	if _, err = m.Delete(ctx, "one"); err != nil {
		t.Fatal(err)
	}
	if _, err = m.Create("one"); err != nil {
		t.Fatal(err)
	}
	oneSession, _ = m.session("one")
	c, err = openStoredDevice(ctx, oneSession.dir, storage)
	if err != nil {
		t.Fatal(err)
	}
	d, err = c.GetFirstDevice(ctx)
	if err != nil || d.ID != nil {
		t.Fatal("deleted credentials reappeared", err)
	}
	c.Close()
	one, _ = m.Inbox("one")
	page, err = one.ListChats("", "", 10)
	if err != nil || len(page.Chats) != 0 {
		t.Fatal("deleted archive reappeared", err)
	}
	for _, id := range []string{"one", "two"} {
		for _, kind := range []string{"device", "inbox"} {
			if _, e := os.Stat(filepath.Join(config.DataDir, id, kind+".db")); !os.IsNotExist(e) {
				t.Fatal("production storage created local SQLite", e)
			}
		}
	}
}

func TestDatabaseWorkerLegacyMigration(t *testing.T) {
	storage, _, engineURL := isolatedDatabaseStorage(t)
	ctx := context.Background()
	storage.root = t.TempDir()
	dir := filepath.Join(storage.root, "legacy")
	legacy, err := NewInbox(dir, 300)
	if err != nil {
		t.Fatal(err)
	}
	msg := MessageView{ChatID: "15551234567@s.whatsapp.net", SenderID: "15551234567@s.whatsapp.net", MessageID: "legacy-test", TimestampMS: 1700000000000, Kind: "text", Text: "Preserved report", Source: "live"}
	if err = legacy.Record([]MessageView{msg}); err != nil {
		t.Fatal(err)
	}
	var extra []MessageView
	for i := 1; i < 250; i++ {
		item := msg
		item.MessageID = fmt.Sprintf("legacy-test-%d", i)
		item.TimestampMS += int64(i)
		extra = append(extra, item)
	}
	if err = legacy.Record(extra); err != nil {
		t.Fatal(err)
	}
	if _, err = legacy.db.Exec("INSERT INTO inbox_cloud_event_markers(sequence,event_key) VALUES(205,'old-marker'); DELETE FROM inbox_cloud_event_markers"); err != nil {
		t.Fatal(err)
	}
	legacy.Close()
	c, err := openDevice(ctx, dir)
	if err != nil {
		t.Fatal(err)
	}
	key := saveSyntheticDevice(t, c, "15550000002")
	c.Close()
	// Simulate the previous library schema and verify that the remote adapter
	// supports its transactional ALTER TABLE upgrade after importing keys.
	oldDevice, err := (localStorage{}).Open(ctx, dir, "device")
	if err != nil {
		t.Fatal(err)
	}
	if _, err = oldDevice.Exec("ALTER TABLE whatsmeow_chat_settings DROP COLUMN wasa_root_secret_id; UPDATE whatsmeow_version SET version=15"); err != nil {
		t.Fatal(err)
	}
	oldDevice.Close()
	for _, kind := range []string{"inbox", "device"} {
		if db, e := storage.Open(ctx, dir, kind); e == nil {
			db.Close()
			t.Fatal("unmigrated legacy data accepted")
		}
		path := filepath.Join(dir, kind+".db")
		source, e := localStorage{}.Open(ctx, dir, kind)
		if e != nil {
			t.Fatal(e)
		}
		// localStorage initialization can update an inbox; the fingerprint is
		// taken by the migrator after these fixture-only writes finish.
		prefix, _ := storage.prefix(dir, kind)
		dest := databasestore.Open(storage.call, storage.db, prefix, 30*time.Second)
		if kind == "inbox" {
			failCall := func(ctx context.Context, id string, value any) (json.RawMessage, error) {
				if p, ok := value.(map[string]any); ok && id == "database::transactionExecute" && strings.HasPrefix(fmt.Sprint(p["sql"]), "INSERT INTO ") && strings.Contains(fmt.Sprint(p["sql"]), prefix+"inbox_messages") {
					return nil, errors.New("synthetic migration interruption")
				}
				return storage.call(ctx, id, value)
			}
			failed := databasestore.Open(failCall, storage.db, prefix, 30*time.Second)
			if _, e = databasestore.Migrate(ctx, source, failed, prefix, path); e == nil {
				t.Fatal("migration interruption was ignored")
			}
			failed.Close()
			var left int
			if e = dest.QueryRowContext(ctx, "SELECT count(*) FROM sqlite_master WHERE type='table' AND substr(name,1,?)=?", len(prefix), prefix).Scan(&left); e != nil || left != 0 {
				t.Fatal("failed migration did not roll back", e)
			}
		}
		count, e := databasestore.Migrate(ctx, source, dest, prefix, path)
		source.Close()
		dest.Close()
		if e != nil || count == 0 {
			t.Fatal("legacy migration failed", e)
		}
		if _, e = os.Stat(path); e != nil {
			t.Fatal("source backup removed", e)
		}
	}
	if command := os.Getenv("WHATSAPP_MIGRATE_BINARY"); command != "" {
		args := []string{"--data-dir", storage.root, "--engine", engineURL, "--namespace", storage.namespace, "--config-name", storage.configID, "--database", storage.db, "--database-namespace", smokeNamespace, "--apply"}
		output, e := exec.Command(command, args...).CombinedOutput()
		if e != nil {
			t.Fatalf("migration command: %v %s", e, output)
		}
		if strings.Count(string(output), "Source preserved.") != 2 {
			t.Fatalf("unexpected migration command result: %s", output)
		}
		lock, e := os.OpenFile(filepath.Join(storage.root, ".worker.lock"), os.O_RDWR, 0600)
		if e != nil {
			t.Fatal(e)
		}
		if e = syscall.Flock(int(lock.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); e != nil {
			lock.Close()
			t.Fatal(e)
		}
		output, e = exec.Command(command, args...).CombinedOutput()
		lock.Close()
		if e == nil || !strings.Contains(string(output), "stop the WhatsApp worker") {
			t.Fatal("migration command did not honor worker lock")
		}
	}
	box, err := newInbox(ctx, storage, dir, 10)
	if err != nil {
		t.Fatal(err)
	}
	page, err := box.ListMessages(msg.ChatID, "", 10)
	if err != nil || len(page.Messages) != 10 || page.Messages[0].Text != msg.Text {
		t.Fatal("migration lost archive", err)
	}
	result, err := box.db.Exec("INSERT INTO inbox_cloud_event_markers(event_key) VALUES(?)", "new-marker")
	if err != nil {
		t.Fatal(err)
	}
	if seq, e := result.LastInsertId(); e != nil || seq != 206 {
		t.Fatal("migration lost AUTOINCREMENT high-water mark", e)
	}
	box.Close()
	c, err = openStoredDevice(ctx, dir, storage)
	if err != nil {
		t.Fatal(err)
	}
	d, err := c.GetFirstDevice(ctx)
	if err != nil || d.ID == nil || !bytes.Equal(d.NoiseKey.Priv[:], key) {
		t.Fatal("migration lost device identity", err)
	}
	c.Close()
	f, err := os.OpenFile(filepath.Join(dir, "inbox.db"), os.O_APPEND|os.O_WRONLY, 0600)
	if err != nil {
		t.Fatal(err)
	}
	f.Write([]byte("changed"))
	f.Close()
	if db, e := storage.Open(ctx, dir, "inbox"); e == nil {
		db.Close()
		t.Fatal("modified legacy backup accepted")
	} else if !strings.Contains(e.Error(), "changed") {
		t.Fatal(e)
	}
}

// Review reproduction: no real engine, network, or remote account is used.
func TestReviewRequestContextReachesStorage(t *testing.T) {
	m, s := testSessionManager(t)
	if err := s.inbox.db.Close(); err != nil {
		t.Fatal(err)
	}
	entered := make(chan context.Context, 1)
	release := make(chan struct{})
	call := func(ctx context.Context, _ string, _ any) (json.RawMessage, error) {
		entered <- ctx
		select {
		case <-release:
			return nil, errors.New("synthetic storage failure")
		case <-ctx.Done():
			return nil, ctx.Err()
		}
	}
	s.inbox.db = databasestore.Open(call, "whatsapp", "wa_review_", 2*time.Second)
	r := NewRegistry(m)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan error, 1)
	go func() {
		_, err := r.Invoke(ctx, "whatsapp::messages::list", map[string]any{"session_id": "test", "chat_id": "15551234567@s.whatsapp.net"})
		done <- err
	}()
	dbctx := <-entered
	cancel()
	<-ctx.Done()
	storageCanceled := dbctx.Err() != nil
	close(release)
	err := <-done
	if !storageCanceled || !errors.Is(err, context.Canceled) {
		t.Fatalf("caller canceled, but in-flight database context canceled=%v and handler returned %v", storageCanceled, err)
	}
}

func TestReviewRejectedReloadPreservesInbox(t *testing.T) {
	m, s := testSessionManager(t)
	before := m.Config()
	messages := []MessageView{
		{ChatID: "15551234567@s.whatsapp.net", SenderID: "15551234567@s.whatsapp.net", MessageID: "first", TimestampMS: 1, Kind: "text", Text: "one", Source: "live"},
		{ChatID: "15551234567@s.whatsapp.net", SenderID: "15551234567@s.whatsapp.net", MessageID: "second", TimestampMS: 2, Kind: "text", Text: "two", Source: "live"},
	}
	if err := s.inbox.Record(messages); err != nil {
		t.Fatal(err)
	}
	// Cloud inboxes are visited after native inboxes, so failure order is deterministic.
	m.cloud["synthetic-closed"] = &Inbox{}
	next := before
	next.MaxInboxMessages = 1
	_, err := m.ApplyConfig(next)
	if err == nil {
		t.Fatal("expected closed Cloud inbox to reject reload")
	}
	page, err := s.inbox.ListMessages(messages[0].ChatID, "", 10)
	if err != nil {
		t.Fatal(err)
	}
	if m.Config().MaxInboxMessages != before.MaxInboxMessages || s.inbox.limit != before.MaxInboxMessages || len(page.Messages) != 2 {
		t.Fatalf("rejected reload retained config=%d, but native inbox limit=%d and messages=%d (want %d and 2)", m.Config().MaxInboxMessages, s.inbox.limit, len(page.Messages), before.MaxInboxMessages)
	}
}
