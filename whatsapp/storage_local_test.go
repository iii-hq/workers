package main

import (
	"context"
	"database/sql"
	"errors"
	"go.mau.fi/whatsmeow/store/sqlstore"
	waLog "go.mau.fi/whatsmeow/util/log"
	_ "modernc.org/sqlite"
	"net/url"
	"os"
	"path/filepath"
	"syscall"
)

// SQLite is linked only into tests and the explicit migration command.
type localStorage struct{}

func (localStorage) Open(ctx context.Context, dir, kind string) (*sql.DB, error) {
	if kind == "device" { // The existing offline device fixture retains its file checks.
		path := filepath.Join(dir, "device.db")
		if err := privateInboxDirectory(dir); err != nil {
			return nil, err
		}
		f, err := os.OpenFile(path, os.O_CREATE|os.O_RDWR|syscall.O_NOFOLLOW, 0600)
		if err != nil {
			return nil, err
		}
		f.Close()
		uri := url.URL{Scheme: "file", Path: path}
		uri.RawQuery = url.Values{"_pragma": {"foreign_keys(ON)", "busy_timeout(5000)", "secure_delete(ON)"}}.Encode()
		db, err := sql.Open("sqlite", uri.String())
		if err == nil {
			db.SetMaxOpenConns(1)
		}
		return db, err
	}
	box, err := NewInbox(dir, 100000)
	if err != nil {
		return nil, err
	}
	return box.db, nil
}
func (localStorage) ClearDevice(ctx context.Context, dir string) error {
	for _, suffix := range []string{"", "-journal", "-wal", "-shm"} {
		if err := os.Remove(filepath.Join(dir, "device.db") + suffix); err != nil && !os.IsNotExist(err) {
			return err
		}
	}
	return nil
}
func newTestManager(c Config) (*Manager, error) { return NewManager(c, localStorage{}) }

func NewInbox(dir string, limit int) (*Inbox, error) {
	if limit < 1 || limit > 100000 {
		return nil, errors.New("max_inbox_messages must be 1..100000")
	}
	if err := privateInboxDirectory(dir); err != nil {
		return nil, err
	}
	path := filepath.Join(dir, "inbox.db")
	for _, suffix := range []string{"", "-journal", "-wal", "-shm"} {
		p := path + suffix
		info, err := os.Lstat(p)
		if err == nil {
			if !info.Mode().IsRegular() {
				return nil, errors.New("inbox files must be regular files")
			}
			if err = os.Chmod(p, 0600); err != nil {
				return nil, err
			}
		} else if !os.IsNotExist(err) {
			return nil, err
		}
	}
	f, err := os.OpenFile(path, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0600)
	if err == nil {
		_ = f.Close()
	} else if !os.IsExist(err) {
		return nil, err
	}
	absolutePath, err := filepath.Abs(path)
	if err != nil {
		return nil, err
	}
	uri := url.URL{Scheme: "file", Path: absolutePath}
	uri.RawQuery = url.Values{"_pragma": {"busy_timeout(5000)", "secure_delete(ON)", "journal_mode(DELETE)"}}.Encode()
	db, err := sql.Open("sqlite", uri.String())
	if err != nil {
		return nil, err
	}
	db.SetMaxOpenConns(1)
	return initializeInbox(db, limit)
}

func openDevice(ctx context.Context, dir string) (*sqlstore.Container, error) {
	path := filepath.Join(dir, "device.db")
	for _, suffix := range []string{"", "-journal", "-wal", "-shm"} {
		p := path + suffix
		if _, err := os.Lstat(p); err == nil {
			if err = regularPrivateFile(p); err != nil {
				return nil, err
			}
		} else if !os.IsNotExist(err) {
			return nil, err
		}
	}
	f, err := os.OpenFile(path, os.O_CREATE|os.O_RDWR|syscall.O_NOFOLLOW, 0600)
	if err != nil {
		return nil, err
	}
	f.Close()
	absolutePath, err := filepath.Abs(path)
	if err != nil {
		return nil, err
	}
	uri := url.URL{Scheme: "file", Path: absolutePath}
	uri.RawQuery = url.Values{"_pragma": {"foreign_keys(ON)", "busy_timeout(5000)", "journal_mode(DELETE)", "secure_delete(ON)"}}.Encode()
	db, err := sql.Open("sqlite", uri.String())
	if err != nil {
		return nil, err
	}
	db.SetMaxOpenConns(1)
	if _, err = db.ExecContext(ctx, "PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000; PRAGMA journal_mode=DELETE; PRAGMA secure_delete=ON"); err != nil {
		db.Close()
		return nil, err
	}
	container := sqlstore.NewWithDB(db, "sqlite3", waLog.Noop)
	if err = container.Upgrade(ctx); err != nil {
		db.Close()
		return nil, err
	}
	return container, nil
}
