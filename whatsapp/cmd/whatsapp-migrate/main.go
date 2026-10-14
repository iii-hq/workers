// whatsapp-migrate is an offline, explicit migration utility. It is the only
// production command that links SQLite; the WhatsApp worker does not.
package main

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io/fs"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/whatsapp/internal/databasestore"
	_ "modernc.org/sqlite"
)

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
func run() error {
	root := flag.String("data-dir", "data/whatsapp-go", "Existing, stopped Go worker data directory")
	engine := flag.String("engine", "ws://127.0.0.1:49134", "iii engine URL")
	namespace := flag.String("namespace", "default", "WhatsApp worker namespace (must match runtime)")
	configID := flag.String("config-name", "whatsapp", "WhatsApp configuration id (must match runtime)")
	dbName := flag.String("database", "whatsapp", "Dedicated database worker pool name")
	dbNamespace := flag.String("database-namespace", "", "Database worker namespace; defaults to WhatsApp namespace")
	apply := flag.Bool("apply", false, "Copy data; without this flag only print the file inventory")
	flag.Parse()
	abs, err := filepath.Abs(*root)
	if err != nil {
		return errors.New("invalid data directory")
	}
	// Reject symlinks in the complete path, not only the final directory.
	for dir := abs; ; dir = filepath.Dir(dir) {
		st, e := os.Lstat(dir)
		if e != nil || !st.IsDir() || st.Mode()&os.ModeSymlink != 0 {
			return errors.New("data directory must exist and contain no symlinks")
		}
		if dir == filepath.Dir(dir) {
			break
		}
	}
	lock, err := os.OpenFile(filepath.Join(abs, ".worker.lock"), os.O_CREATE|os.O_RDWR|syscall.O_NOFOLLOW, 0600)
	if err != nil {
		return errors.New("cannot acquire worker storage lock")
	}
	defer lock.Close()
	if err = syscall.Flock(int(lock.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		return errors.New("stop the WhatsApp worker before inspecting or migrating storage")
	}
	var files []string
	err = filepath.WalkDir(abs, func(path string, entry fs.DirEntry, walkErr error) error {
		if walkErr != nil {
			return errors.New("cannot inspect legacy storage")
		}
		if entry.Type()&os.ModeSymlink != 0 {
			return errors.New("legacy storage contains a symlink")
		}
		if !entry.IsDir() && strings.HasSuffix(entry.Name(), ".db") {
			rel, e := filepath.Rel(abs, path)
			if e != nil {
				return e
			}
			rel = filepath.ToSlash(rel)
			if _, _, e = databasestore.Scope(rel); e != nil {
				return e
			}
			files = append(files, rel)
		}
		return nil
	})
	if err != nil {
		return err
	}
	for _, rel := range files {
		fmt.Println("Legacy store:", rel)
	}
	if !*apply {
		fmt.Println("Preview only. Stop the worker, back up data_dir, configure a dedicated SQLite pool (max=1), then repeat with --apply.")
		return nil
	}
	if len(files) == 0 {
		return errors.New("no legacy databases to migrate")
	}
	if err = os.Setenv("III_DISABLE_TRACE_PAYLOADS", "1"); err != nil {
		return errors.New("cannot disable migration payload traces")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Minute)
	defer cancel()
	client := iii.New(*engine, iii.WithName("whatsapp-migrate"))
	defer client.Close()
	connectCtx, connectCancel := context.WithTimeout(ctx, 15*time.Second)
	err = client.Connect(connectCtx)
	connectCancel()
	if err != nil {
		return errors.New("cannot connect to iii engine")
	}
	ns := *dbNamespace
	if ns == "" {
		ns = *namespace
	}
	call := func(ctx context.Context, id string, value any) (json.RawMessage, error) {
		data, e := json.Marshal(value)
		if e != nil {
			return nil, errors.New("invalid migration request")
		}
		return client.Trigger(ctx, iii.TriggerRequest{FunctionID: id, Namespace: ns, Data: data, Timeout: 30 * time.Second})
	}
	if err = databasestore.Validate(ctx, call, *dbName, 30*time.Second); err != nil {
		return err
	}
	for _, rel := range files {
		scope, kind, _ := databasestore.Scope(rel)
		prefix := databasestore.Prefix(*namespace, *configID, scope, kind)
		dest := databasestore.Open(call, *dbName, prefix, 5*time.Minute)
		path := filepath.Join(abs, filepath.FromSlash(rel))
		uri := url.URL{Scheme: "file", Path: path}
		uri.RawQuery = url.Values{"mode": {"ro"}}.Encode()
		source, e := sql.Open("sqlite", uri.String())
		if e != nil {
			dest.Close()
			return errors.New("cannot open legacy source")
		}
		source.SetMaxOpenConns(1)
		count, e := databasestore.Migrate(ctx, source, dest, prefix, path)
		source.Close()
		dest.Close()
		if e != nil {
			return fmt.Errorf("migration stopped for %s: %w", rel, e)
		}
		fmt.Printf("Migrated %s: %d rows (or already migrated). Source preserved.\n", rel, count)
	}
	return nil
}
