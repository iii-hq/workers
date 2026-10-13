package databasestore

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"os"
	"strings"
)

func fingerprint(path string) (string, error) {
	info, err := os.Lstat(path)
	if os.IsNotExist(err) {
		return "", nil
	}
	if err != nil {
		return "", errors.New("cannot inspect legacy storage")
	}
	if !info.Mode().IsRegular() {
		return "", errors.New("legacy storage must be a regular file")
	}
	if info.Size() == 0 {
		return "", nil
	}
	for _, suffix := range []string{"-wal", "-journal"} {
		if st, e := os.Lstat(path + suffix); e == nil && st.Size() > 0 {
			return "", errors.New("legacy storage has an active journal; stop the old worker and checkpoint before migrating")
		} else if e != nil && !os.IsNotExist(e) {
			return "", errors.New("cannot inspect legacy journal")
		}
	}
	f, err := os.Open(path)
	if err != nil {
		return "", errors.New("cannot read legacy storage")
	}
	defer f.Close()
	h := sha256.New()
	if _, err = io.Copy(h, f); err != nil {
		return "", errors.New("cannot fingerprint legacy storage")
	}
	return hex.EncodeToString(h.Sum(nil)), nil
}

// CheckLegacy prevents a new empty store from silently replacing paired keys
// or captured history. The source file remains an immutable recovery copy.
func CheckLegacy(ctx context.Context, db *sql.DB, path string) error {
	hash, err := fingerprint(path)
	if err != nil || hash == "" {
		return err
	}
	var exists bool
	if err = db.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND tbl_name=?)", "storage_migration").Scan(&exists); err != nil {
		return err
	}
	if !exists {
		return errors.New("legacy SQLite data requires the whatsapp-migrate command before starting this worker")
	}
	var saved string
	if err = db.QueryRowContext(ctx, "SELECT source_sha256 FROM storage_migration WHERE id=1").Scan(&saved); err != nil {
		return errors.New("legacy storage migration was not confirmed")
	}
	if saved != hash {
		return errors.New("legacy SQLite data changed after migration; stop and reconcile the stores before starting")
	}
	return nil
}

type migrationTable struct{ Name, SQL string }

// Migrate copies a stopped, read-only source into an empty scoped store in one
// transaction. Values remain bound parameters, including all native keys.
func Migrate(ctx context.Context, source, dest *sql.DB, prefix, path string) (int64, error) {
	hash, err := fingerprint(path)
	if err != nil {
		return 0, err
	}
	if hash == "" {
		return 0, errors.New("legacy storage is empty")
	}
	if CheckLegacy(ctx, dest, path) == nil {
		return 0, nil
	}
	var count int
	if err = dest.QueryRowContext(ctx, "SELECT count(*) FROM sqlite_master WHERE type='table' AND substr(name,1,?)=?", len(prefix), prefix).Scan(&count); err != nil {
		return 0, err
	}
	if count != 0 {
		return 0, errors.New("migration destination is not empty; refusing to overwrite existing storage")
	}
	r, err := source.QueryContext(ctx, "SELECT name,sql FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY rowid")
	if err != nil {
		return 0, errors.New("cannot inspect source schema")
	}
	var tables []migrationTable
	for r.Next() {
		var t migrationTable
		if err = r.Scan(&t.Name, &t.SQL); err != nil {
			r.Close()
			return 0, errors.New("invalid source schema")
		}
		if !owned(t.Name) || t.Name == "storage_migration" {
			r.Close()
			return 0, errors.New("source contains unexpected tables")
		}
		tables = append(tables, t)
	}
	err = r.Err()
	r.Close()
	if err != nil || len(tables) == 0 {
		return 0, errors.New("legacy source has no usable schema")
	}
	tx, err := dest.BeginTx(ctx, nil)
	if err != nil {
		return 0, err
	}
	defer tx.Rollback()
	if _, err = tx.ExecContext(ctx, "PRAGMA defer_foreign_keys=ON; PRAGMA secure_delete=ON"); err != nil {
		return 0, err
	}
	var copied int64
	for _, t := range tables {
		if _, err = tx.ExecContext(ctx, t.SQL); err != nil {
			return 0, err
		}
		r, err = source.QueryContext(ctx, "SELECT * FROM "+quote(t.Name))
		if err != nil {
			return 0, errors.New("cannot read source rows")
		}
		cols, err := r.Columns()
		if err != nil {
			r.Close()
			return 0, err
		}
		quoted := make([]string, len(cols))
		holders := make([]string, len(cols))
		for i, col := range cols {
			quoted[i] = quote(col)
			holders[i] = "?"
		}
		insert := "INSERT INTO " + quote(t.Name) + " (" + strings.Join(quoted, ",") + ") VALUES (" + strings.Join(holders, ",") + ")"
		var tableCount int64
		maxRows := 900 / len(cols)
		if maxRows > 100 {
			maxRows = 100
		}
		if maxRows < 1 {
			r.Close()
			return 0, errors.New("source table has too many columns")
		}
		var bound []any
		var groups []string
		batchBytes := 0
		flush := func() error {
			if len(groups) == 0 {
				return nil
			}
			sql := insert[:strings.Index(insert, " VALUES (")] + " VALUES " + strings.Join(groups, ",")
			_, e := tx.ExecContext(ctx, sql, bound...)
			bound = nil
			groups = nil
			batchBytes = 0
			return e
		}
		for r.Next() {
			values := make([]any, len(cols))
			pointers := make([]any, len(cols))
			for i := range values {
				pointers[i] = &values[i]
			}
			if err = r.Scan(pointers...); err != nil {
				r.Close()
				return 0, errors.New("cannot decode source row")
			}
			size := len(values) * 24
			for _, v := range values {
				switch value := v.(type) {
				case string:
					size += len(value) * 6
				case []byte:
					size += len(value) * 2
				}
			}
			if len(groups) > 0 && (len(groups) >= maxRows || batchBytes+size > 256*1024) {
				if err = flush(); err != nil {
					r.Close()
					return 0, err
				}
			}
			// One oversized row cannot be split without changing the native schema.
			if size > 1024*1024 {
				r.Close()
				return 0, errors.New("legacy row exceeds the 1 MiB migration transfer limit")
			}
			bound = append(bound, values...)
			groups = append(groups, "("+strings.Join(holders, ",")+")")
			batchBytes += size
			copied++
			tableCount++
		}
		err = r.Err()
		r.Close()
		if err != nil {
			return 0, errors.New("cannot read source rows")
		}
		if err = flush(); err != nil {
			return 0, err
		}

		var actual int64
		if err = tx.QueryRowContext(ctx, "SELECT count(*) FROM "+quote(t.Name)).Scan(&actual); err != nil {
			return 0, err
		}
		if actual != tableCount {
			return 0, errors.New("migration row count mismatch")
		}
	}
	r, err = source.QueryContext(ctx, "SELECT sql FROM sqlite_master WHERE type='index' AND sql IS NOT NULL")
	if err != nil {
		return 0, errors.New("cannot read source indexes")
	}
	for r.Next() {
		var schema string
		if err = r.Scan(&schema); err != nil {
			r.Close()
			return 0, errors.New("invalid source index")
		}
		if _, err = tx.ExecContext(ctx, schema); err != nil {
			r.Close()
			return 0, err
		}
	}
	err = r.Err()
	r.Close()
	if err != nil {
		return 0, errors.New("cannot read source indexes")
	}
	// Preserve high water marks after pruning/deleting AUTOINCREMENT rows.
	var hasSequence bool
	if err = source.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='sqlite_sequence')").Scan(&hasSequence); err != nil {
		return 0, errors.New("cannot read source sequence metadata")
	}
	if hasSequence {
		r, err = source.QueryContext(ctx, "SELECT name,seq FROM sqlite_sequence")
		if err != nil {
			return 0, errors.New("cannot read source sequences")
		}
		for r.Next() {
			var name string
			var seq int64
			if err = r.Scan(&name, &seq); err != nil {
				r.Close()
				return 0, errors.New("invalid source sequence")
			}
			if !owned(name) {
				r.Close()
				return 0, errors.New("unexpected source sequence")
			}
			if _, err = tx.ExecContext(ctx, "DELETE FROM sqlite_sequence WHERE name=?", prefix+name); err == nil {
				_, err = tx.ExecContext(ctx, "INSERT INTO sqlite_sequence(name,seq) VALUES(?,?)", prefix+name, seq)
			}
			if err != nil {
				r.Close()
				return 0, err
			}
		}
		err = r.Err()
		r.Close()
		if err != nil {
			return 0, errors.New("cannot read source sequences")
		}
	}
	if _, err = tx.ExecContext(ctx, "CREATE TABLE storage_migration(id INTEGER PRIMARY KEY CHECK(id=1),source_sha256 TEXT NOT NULL)"); err != nil {
		return 0, err
	}
	if _, err = tx.ExecContext(ctx, "INSERT INTO storage_migration(id,source_sha256) VALUES(1,?)", hash); err != nil {
		return 0, err
	}

	// Stop if the source changed while it was being copied.
	after, err := fingerprint(path)
	if err != nil || after != hash {
		return 0, errors.New("legacy source changed during migration")
	}
	if err = tx.Commit(); err != nil {
		return 0, err
	}
	return copied, nil
}

// Clear removes every session-owned row, including maps that do not reference
// the account via a foreign key. Schema and migration provenance remain.
func Clear(ctx context.Context, db *sql.DB, prefix, tablePrefix string) error {
	r, err := db.QueryContext(ctx, "SELECT name FROM sqlite_master WHERE type='table' AND substr(name,1,?)=?", len(prefix), prefix)
	if err != nil {
		return err
	}
	var names []string
	for r.Next() {
		var name string
		if err = r.Scan(&name); err != nil {
			r.Close()
			return err
		}
		local := strings.TrimPrefix(name, prefix)
		if strings.HasPrefix(local, tablePrefix) && local != "whatsmeow_version" {
			names = append(names, local)
		}
	}
	err = r.Err()
	r.Close()
	if err != nil {
		return err
	}
	tx, err := db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer tx.Rollback()
	if _, err = tx.ExecContext(ctx, "PRAGMA defer_foreign_keys=ON; PRAGMA secure_delete=ON"); err != nil {
		return err
	}
	for _, name := range names {
		if _, err = tx.ExecContext(ctx, "DELETE FROM "+quote(name)); err != nil {
			return err
		}
	}
	return tx.Commit()
}
func quote(name string) string { return `"` + strings.ReplaceAll(name, `"`, `""`) + `"` }
func Scope(path string) (string, string, error) {
	parts := strings.Split(strings.ReplaceAll(path, "\\", "/"), "/")
	if len(parts) == 2 && (parts[1] == "inbox.db" || parts[1] == "device.db") && validSession(parts[0]) {
		return "session:" + parts[0], strings.TrimSuffix(parts[1], ".db"), nil
	}
	if len(parts) == 3 && parts[0] == ".cloud" && parts[1] != "" && strings.Trim(parts[1], "0123456789") == "" && parts[2] == "inbox.db" {
		return "cloud:" + parts[1], "inbox", nil
	}
	return "", "", fmt.Errorf("invalid legacy storage path")
}

func validSession(id string) bool {
	if len(id) < 1 || len(id) > 64 {
		return false
	}
	for i := range id {
		if !letter(id[i]) && !digit(id[i]) && id[i] != '-' {
			return false
		}
	}
	return true
}
