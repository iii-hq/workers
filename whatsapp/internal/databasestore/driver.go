// Package databasestore adapts the database worker's SQLite contracts to
// database/sql. Each store uses its own deterministic table prefix.
package databasestore

import (
	"bytes"
	"context"
	"crypto/sha256"
	"database/sql"
	"database/sql/driver"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"strconv"
	"strings"
	"time"
)

type Call func(context.Context, string, any) (json.RawMessage, error)

// Prefix is stable across restarts, independent of host filesystem paths.
func Prefix(namespace, configID, scope, kind string) string {
	sum := sha256.Sum256([]byte(namespace + "\x00" + configID + "\x00" + scope + "\x00" + kind))
	return "wa_" + hex.EncodeToString(sum[:16]) + "_"
}

type connector struct {
	call       Call
	db, prefix string
	timeout    time.Duration
}
type connection struct {
	*connector
	tx        string
	txContext context.Context
	broken    bool
}
type statement struct {
	c     *connection
	query string
}
type transaction struct{ c *connection }
type result struct{ affected, inserted int64 }
type rows struct {
	columns []column
	values  []map[string]any
	offset  int
}
type column struct {
	Name string `json:"name"`
	Type string `json:"type"`
}

// Open never loads a local SQL driver or retries failed mutations.
func Open(call Call, db, prefix string, timeout time.Duration) *sql.DB {
	dbConn := sql.OpenDB(&connector{call, db, prefix, timeout})
	dbConn.SetMaxOpenConns(1)
	return dbConn
}
func (c *connector) Connect(context.Context) (driver.Conn, error) {
	return &connection{connector: c}, nil
}
func (c *connector) Driver() driver.Driver { return remoteDriver{} }

type remoteDriver struct{}

func (remoteDriver) Open(string) (driver.Conn, error) {
	return nil, errors.New("use databasestore.Open")
}
func (c *connection) Close() error {
	if c.tx != "" {
		return c.finish(false)
	}
	return nil
}
func (c *connection) IsValid() bool                             { return !c.broken }
func (c *connection) Prepare(query string) (driver.Stmt, error) { return &statement{c, query}, nil }
func (c *connection) Begin() (driver.Tx, error) {
	return c.BeginTx(context.Background(), driver.TxOptions{})
}
func (c *connection) BeginTx(ctx context.Context, opts driver.TxOptions) (driver.Tx, error) {
	if c.broken || c.tx != "" {
		return nil, errors.New("database connection is unavailable")
	}
	if opts.ReadOnly || (opts.Isolation != 0 && opts.Isolation != driver.IsolationLevel(sql.LevelSerializable)) {
		return nil, errors.New("unsupported database transaction options")
	}
	// A pooled connection may have been replaced since Open. Foreign keys
	// must be enabled before BEGIN; this dedicated pool has one connection.
	if err := c.invoke(ctx, "execute", map[string]any{"db": c.db, "sql": "PRAGMA foreign_keys=ON"}, nil); err != nil {
		return nil, err
	}
	var response struct {
		Transaction struct {
			ID string `json:"id"`
		} `json:"transaction"`
	}
	lease := c.timeout
	if deadline, ok := ctx.Deadline(); ok {
		lease = min(lease, time.Until(deadline))
	}
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if err := c.invoke(ctx, "beginTransaction", map[string]any{"db": c.db, "isolation": "serializable", "timeout_ms": max(int64(1), lease.Milliseconds())}, &response); err != nil {
		return nil, err
	}
	if response.Transaction.ID == "" {
		c.broken = true
		return nil, errors.New("database worker returned an invalid transaction handle")
	}
	c.tx = response.Transaction.ID
	c.txContext = ctx
	if _, err := c.ExecContext(ctx, "PRAGMA secure_delete=ON; PRAGMA busy_timeout=5000", nil); err != nil {
		_ = c.finish(false)
		return nil, err
	}
	return &transaction{c}, nil
}
func (t *transaction) Commit() error   { return t.c.finish(true) }
func (t *transaction) Rollback() error { return t.c.finish(false) }
func (c *connection) finish(commit bool) error {
	if c.tx == "" {
		return errors.New("database transaction has already finished")
	}
	id := c.tx
	c.tx = ""
	var response struct {
		Committed  bool `json:"committed"`
		RolledBack bool `json:"rolled_back"`
	}
	var err error
	if commit {
		err = c.invoke(c.txContext, "commitTransaction", map[string]any{"transaction_id": id}, &response)
	} else {
		err = c.rollback(id, &response)
	}
	if err == nil && ((commit && !response.Committed) || (!commit && !response.RolledBack)) {
		err = errors.New("database worker did not confirm transaction finalization")
	}
	if err != nil {
		c.broken = true
		if commit {
			// An acknowledgement may be lost after commit. Never replay writes.
			_ = c.rollback(id, nil)
			return fmt.Errorf("database commit was not confirmed; outcome may be unknown; no retry: %w", err)
		}
	}
	return err
}

// Rollback needs a fresh cancellation budget to release a canceled transaction,
// but cleanup must not hold an expired request for the full storage timeout.
func (c *connection) rollback(id string, out any) error {
	ctx, cancel := context.WithTimeout(context.WithoutCancel(c.txContext), min(c.timeout, time.Second))
	defer cancel()
	return c.invoke(ctx, "rollbackTransaction", map[string]any{"transaction_id": id}, out)
}

func (c *connection) invoke(ctx context.Context, fn string, request, out any) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	ctx, cancel := context.WithTimeout(ctx, c.timeout)
	defer cancel()
	raw, err := c.call(ctx, "database::"+fn, request)
	if err != nil {
		// Database errors can contain SQL, row values, or crypto material.
		// Keep the cause only for cancellation classification.
		if errors.Is(err, context.DeadlineExceeded) || errors.Is(ctx.Err(), context.DeadlineExceeded) {
			return fmt.Errorf("database worker request timed out; outcome may be unknown; no retry: %w", context.DeadlineExceeded)
		}
		if ctx.Err() != nil {
			return ctx.Err()
		}
		return errors.New("database worker request failed; details suppressed to protect stored data")
	}
	if err := ctx.Err(); err != nil {
		return err
	}
	if out == nil {
		return nil
	}
	d := json.NewDecoder(bytes.NewReader(raw))
	d.UseNumber()
	if err = d.Decode(out); err != nil {
		return errors.New("invalid database worker response")
	}
	return nil
}
func (c *connection) payload(query string, args []driver.NamedValue) (map[string]any, error) {
	query, params, err := rewrite(query, c.prefix, args)
	if err != nil {
		return nil, err
	}
	p := map[string]any{"sql": query, "params": params}
	if c.tx != "" {
		p["transaction_id"] = c.tx
	} else {
		p["db"] = c.db
	}
	return p, nil
}
func (c *connection) ExecContext(ctx context.Context, query string, args []driver.NamedValue) (driver.Result, error) {
	if c.broken {
		return nil, errors.New("database connection is unavailable")
	}
	parts, err := splitSQL(query)
	if err != nil {
		return nil, err
	}
	if len(parts) > 1 && len(args) > 0 {
		return nil, errors.New("parameters require one SQL statement")
	}
	var last result
	for _, part := range parts {
		p, err := c.payload(part, args)
		if err != nil {
			return nil, err
		}
		fn := "execute"
		if c.tx != "" {
			fn = "transactionExecute"
		} else if !strings.HasPrefix(strings.ToUpper(strings.TrimSpace(part)), "PRAGMA ") {
			if err := c.invoke(ctx, "execute", map[string]any{"db": c.db, "sql": "PRAGMA secure_delete=ON"}, nil); err != nil {
				return nil, err
			}
		}
		var response struct {
			Affected int64   `json:"affected_rows"`
			Inserted *string `json:"last_insert_id"`
		}
		if err = c.invoke(ctx, fn, p, &response); err != nil {
			return nil, err
		}
		last.affected = response.Affected
		last.inserted = 0
		if response.Inserted != nil {
			last.inserted, err = strconv.ParseInt(*response.Inserted, 10, 64)
			if err != nil {
				return nil, errors.New("invalid database insert ID")
			}
		}
	}
	return last, nil
}
func (c *connection) QueryContext(ctx context.Context, query string, args []driver.NamedValue) (driver.Rows, error) {
	if c.broken {
		return nil, errors.New("database connection is unavailable")
	}
	parts, err := splitSQL(query)
	if err != nil || len(parts) != 1 {
		return nil, errors.New("queries require one SQL statement")
	}
	p, err := c.payload(parts[0], args)
	if err != nil {
		return nil, err
	}
	fn := "query"
	if c.tx != "" {
		fn = "transactionQuery"
	}
	var response struct {
		Columns []column         `json:"columns"`
		Rows    []map[string]any `json:"rows"`
	}
	if err = c.invoke(ctx, fn, p, &response); err != nil {
		return nil, err
	}
	return &rows{columns: response.Columns, values: response.Rows}, nil
}
func (s *statement) Close() error  { return nil }
func (s *statement) NumInput() int { return -1 }
func (s *statement) Exec(values []driver.Value) (driver.Result, error) {
	return s.c.ExecContext(context.Background(), s.query, named(values))
}
func (s *statement) Query(values []driver.Value) (driver.Rows, error) {
	return s.c.QueryContext(context.Background(), s.query, named(values))
}
func (s *statement) ExecContext(ctx context.Context, values []driver.NamedValue) (driver.Result, error) {
	return s.c.ExecContext(ctx, s.query, values)
}
func (s *statement) QueryContext(ctx context.Context, values []driver.NamedValue) (driver.Rows, error) {
	return s.c.QueryContext(ctx, s.query, values)
}
func named(values []driver.Value) []driver.NamedValue {
	out := make([]driver.NamedValue, len(values))
	for i, v := range values {
		out[i] = driver.NamedValue{Ordinal: i + 1, Value: v}
	}
	return out
}
func (r result) LastInsertId() (int64, error) { return r.inserted, nil }
func (r result) RowsAffected() (int64, error) { return r.affected, nil }
func (r *rows) Columns() []string {
	out := make([]string, len(r.columns))
	for i, c := range r.columns {
		out[i] = c.Name
	}
	return out
}
func (r *rows) Close() error { r.values = nil; return nil }
func (r *rows) Next(dest []driver.Value) error {
	if r.offset >= len(r.values) {
		return io.EOF
	}
	row := r.values[r.offset]
	r.offset++
	for i, c := range r.columns {
		v, ok := row[c.Name]
		if !ok {
			return errors.New("database worker returned an incomplete row")
		}
		switch value := v.(type) {
		case nil:
			dest[i] = nil
		case bool:
			dest[i] = value
		case json.Number:
			if strings.ContainsAny(value.String(), ".eE") {
				f, e := value.Float64()
				if e != nil {
					return errors.New("invalid database number")
				}
				dest[i] = f
			} else {
				n, e := value.Int64()
				if e != nil {
					return errors.New("database integer exceeds int64")
				}
				dest[i] = n
			}
		case string:
			if strings.EqualFold(c.Type, "bytea") || strings.EqualFold(c.Type, "blob") {
				b, e := base64.StdEncoding.DecodeString(value)
				if e != nil {
					return errors.New("invalid database binary value")
				}
				dest[i] = b
			} else {
				dest[i] = value
			}
		default:
			return errors.New("unsupported database value")
		}
	}
	return nil
}

// Validate requires a dedicated, single-connection SQLite pool. This keeps
// PRAGMA settings stable for the library's schema upgrades.
func Validate(ctx context.Context, call Call, db string, timeout time.Duration) error {
	c := &connection{connector: &connector{call: call, db: db, timeout: timeout}}
	var response struct {
		Databases []struct {
			Name, Driver string
			Pool         struct{ Max int }
		}
	}
	if err := c.invoke(ctx, "listDatabases", map[string]any{}, &response); err != nil {
		return err
	}
	for _, entry := range response.Databases {
		if entry.Name == db {
			if entry.Driver != "sqlite" || entry.Pool.Max != 1 {
				return errors.New("WhatsApp requires a dedicated SQLite database worker pool with max=1")
			}
			return nil
		}
	}
	return fmt.Errorf("configure database %q in the database worker before starting WhatsApp", db)
}
