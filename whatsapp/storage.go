package main

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/whatsapp/internal/databasestore"
)

type storageBackend interface {
	Open(context.Context, string, string) (*sql.DB, error)
	ClearDevice(context.Context, string) error
}
type databaseStorage struct {
	call                          databasestore.Call
	db, root, namespace, configID string
	timeout                       time.Duration
}

func databaseCall(client *iii.Client, namespace string, timeout time.Duration) databasestore.Call {
	return func(ctx context.Context, id string, value any) (json.RawMessage, error) {
		payload, err := json.Marshal(value)
		if err != nil {
			return nil, errors.New("invalid database request")
		}
		return client.Trigger(ctx, iii.TriggerRequest{FunctionID: id, Namespace: namespace, Data: payload, Timeout: timeout})
	}
}
func newDatabaseStorage(ctx context.Context, client *iii.Client, config Config) (*databaseStorage, error) {
	namespace := config.Database.Namespace
	if namespace == "" {
		namespace = Namespace()
	}
	timeout := time.Duration(config.OperationTimeoutSecs) * time.Second
	call := databaseCall(client, namespace, timeout)
	if err := databasestore.Validate(ctx, call, config.Database.Name, timeout); err != nil {
		return nil, err
	}
	return &databaseStorage{call: call, db: config.Database.Name, root: config.DataDir, namespace: Namespace(), configID: ConfigID(), timeout: timeout}, nil
}
func (s *databaseStorage) prefix(dir, kind string) (string, error) {
	rel, err := filepath.Rel(s.root, dir)
	if err != nil || rel == "." || strings.HasPrefix(rel, "..") || filepath.IsAbs(rel) {
		return "", errors.New("storage path is outside data_dir")
	}
	parts := strings.Split(filepath.ToSlash(rel), "/")
	scope := ""
	if len(parts) == 1 && validSessionID(parts[0]) == nil {
		scope = "session:" + parts[0]
	}
	if len(parts) == 2 && parts[0] == ".cloud" && parts[1] != "" && strings.Trim(parts[1], "0123456789") == "" && kind == "inbox" {
		scope = "cloud:" + parts[1]
	}
	if scope == "" || (kind != "inbox" && kind != "device") {
		return "", errors.New("invalid storage scope")
	}
	return databasestore.Prefix(s.namespace, s.configID, scope, kind), nil
}
func (s *databaseStorage) Open(ctx context.Context, dir, kind string) (*sql.DB, error) {
	prefix, err := s.prefix(dir, kind)
	if err != nil {
		return nil, err
	}
	if err = privateInboxDirectory(dir); err != nil {
		return nil, err
	}
	db := databasestore.Open(s.call, s.db, prefix, s.timeout)
	if err = databasestore.CheckLegacy(ctx, db, filepath.Join(dir, kind+".db")); err != nil {
		db.Close()
		return nil, err
	}
	// These statements target the database worker's single SQLite connection.
	if _, err = db.ExecContext(ctx, "PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000; PRAGMA secure_delete=ON"); err != nil {
		db.Close()
		return nil, err
	}
	var probe string
	if err = db.QueryRowContext(ctx, "SELECT hex(unhex(?))", "00FF").Scan(&probe); err != nil || probe != "00FF" {
		db.Close()
		return nil, errors.New("database worker requires SQLite 3.41 or later with unhex support")
	}
	return db, nil
}
func (s *databaseStorage) ClearDevice(ctx context.Context, dir string) error {
	prefix, err := s.prefix(dir, "device")
	if err != nil {
		return err
	}
	db := databasestore.Open(s.call, s.db, prefix, s.timeout)
	defer db.Close()
	if err = databasestore.CheckLegacy(ctx, db, filepath.Join(dir, "device.db")); err != nil {
		return err
	}
	if err = databasestore.Clear(ctx, db, prefix, "whatsmeow_"); err != nil {
		return err
	}
	// Logout/delete also retires the migrated local credential backup.
	for _, suffix := range []string{"", "-journal", "-wal", "-shm"} {
		path := filepath.Join(dir, "device.db") + suffix
		if _, err = os.Lstat(path); os.IsNotExist(err) {
			continue
		}
		if err != nil {
			return errors.New("cannot inspect legacy credential backup")
		}
		if err = regularPrivateFile(path); err != nil {
			return err
		}
		if err = os.Remove(path); err != nil {
			return errors.New("cannot remove legacy credential backup")
		}
	}
	return nil
}
