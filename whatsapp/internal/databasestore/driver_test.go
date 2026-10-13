package databasestore

import (
	"context"
	"database/sql/driver"
	"encoding/json"
	"errors"
	"fmt"
	"reflect"
	"strings"
	"testing"
	"time"
)

func TestSQLScopeAndBinaryParameters(t *testing.T) {
	query, params, err := rewrite("INSERT INTO whatsmeow_sessions(our_jid,their_id,session) VALUES($1,$2,$3) ON CONFLICT(our_jid,their_id) DO UPDATE SET session=$3", "wa_scope_", []driver.NamedValue{{Ordinal: 1, Value: "inbox_messages"}, {Ordinal: 2, Value: "peer"}, {Ordinal: 3, Value: []byte{0, 255, 42}}})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(query, "wa_scope_whatsmeow_sessions") || strings.Count(query, "unhex(?3)") != 2 || strings.Contains(query, "00ff2a") {
		t.Fatalf("bad scoped SQL: %s", query)
	}
	if !reflect.DeepEqual(params, []any{"inbox_messages", "peer", "00ff2a"}) {
		t.Fatalf("wrong parameters: %+v", params)
	}
	query, _, err = rewrite("SELECT 'inbox_messages;whatsmeow_device', \"inbox_messages\".text FROM inbox_messages -- whatsmeow_device\n WHERE text=?", "wa_scope_", []driver.NamedValue{{Ordinal: 1, Value: "hello"}})
	if err != nil || !strings.Contains(query, "'inbox_messages;whatsmeow_device'") || !strings.Contains(query, "\"wa_scope_inbox_messages\".text") || !strings.Contains(query, "-- whatsmeow_device") {
		t.Fatalf("quote/comment rewriting: %s %v", query, err)
	}
	query, params, err = rewrite("SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name=?2)", "wa_scope_", []driver.NamedValue{{Ordinal: 1, Value: "whatsmeow_version"}, {Ordinal: 2, Value: "version"}})
	if err != nil || params[0] != "wa_scope_whatsmeow_version" {
		t.Fatalf("bound schema lookup: %s %+v %v", query, params, err)
	}
	query, _, err = rewrite("SELECT * FROM pragma_table_info('inbox_external_receipts')", "wa_scope_", nil)
	if err != nil || !strings.Contains(query, "'wa_scope_inbox_external_receipts'") {
		t.Fatal(query, err)
	}
	parts, err := splitSQL("-- migration\nCREATE TABLE inbox_meta(text TEXT); INSERT INTO inbox_meta VALUES('a;b'); /* trailing; */")
	if err != nil || len(parts) != 2 {
		t.Fatalf("statement splitting: %+v %v", parts, err)
	}
	for _, q := range []string{"SELECT 'broken", "SELECT /*broken", "SELECT ?2"} {
		if _, _, err = rewrite(q, "wa_scope_", nil); err == nil {
			t.Fatalf("accepted %q", q)
		}
	}
	if Prefix("one", "cfg", "session:a", "device") == Prefix("two", "cfg", "session:a", "device") || Prefix("one", "cfg", "session:a", "device") == Prefix("one", "cfg", "session:a", "inbox") {
		t.Fatal("storage scopes collide")
	}
}
func TestRowsPreserveBinaryAndIntegerPrecision(t *testing.T) {
	call := func(context.Context, string, any) (json.RawMessage, error) {
		return json.RawMessage(`{"columns":[{"name":"key","type":"bytea"},{"name":"seq","type":"BIGINT"}],"rows":[{"key":"AP8q","seq":9007199254740993}]}`), nil
	}
	db := Open(call, "whatsapp", "wa_test_", time.Second)
	defer db.Close()
	var key []byte
	var seq int64
	if err := db.QueryRow("SELECT identity,seq FROM whatsmeow_identity_keys").Scan(&key, &seq); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(key, []byte{0, 255, 42}) || seq != 9007199254740993 {
		t.Fatalf("corrupted stored values: %v %d", key, seq)
	}
}
func TestTransactionsDoNotRetryUnknownCommitOrLeakErrors(t *testing.T) {
	var methods []string
	call := func(_ context.Context, id string, req any) (json.RawMessage, error) {
		methods = append(methods, id)
		if p, ok := req.(map[string]any); ok && strings.HasPrefix(fmt.Sprint(p["sql"]), "PRAGMA ") {
			return json.RawMessage(`{"affected_rows":0}`), nil
		}
		switch id {
		case "database::beginTransaction":
			return json.RawMessage(`{"transaction":{"id":"tx-test"}}`), nil
		case "database::transactionExecute":
			return json.RawMessage(`{"affected_rows":1,"last_insert_id":"42"}`), nil
		case "database::commitTransaction":
			return nil, context.DeadlineExceeded
		case "database::rollbackTransaction":
			return json.RawMessage(`{"rolled_back":true}`), nil
		default:
			return nil, errors.New("secret identity key and token")
		}
	}
	db := Open(call, "whatsapp", "wa_test_", time.Second)
	defer db.Close()
	tx, err := db.BeginTx(context.Background(), nil)
	if err != nil {
		t.Fatal(err)
	}
	result, err := tx.Exec("INSERT INTO inbox_meta(id) VALUES(?)", 42)
	if err != nil {
		t.Fatal(err)
	}
	if id, _ := result.LastInsertId(); id != 42 {
		t.Fatal(id)
	}
	err = tx.Commit()
	if err == nil || !strings.Contains(err.Error(), "outcome may be unknown") {
		t.Fatal(err)
	}
	if strings.Count(strings.Join(methods, ","), "database::commitTransaction") != 1 || strings.Count(strings.Join(methods, ","), "database::transactionExecute") != 3 {
		t.Fatalf("mutation was retried: %+v", methods)
	}
	_, err = db.Exec("DELETE FROM inbox_meta")
	if err == nil || strings.Contains(err.Error(), "secret") {
		t.Fatalf("unsafe storage error: %v", err)
	}
}
func TestValidateRejectsWrongPool(t *testing.T) {
	for _, raw := range []string{`{"databases":[]}`, `{"databases":[{"name":"whatsapp","driver":"postgres","pool":{"max":1}}]}`, `{"databases":[{"name":"whatsapp","driver":"sqlite","pool":{"max":2}}]}`} {
		call := func(context.Context, string, any) (json.RawMessage, error) { return json.RawMessage(raw), nil }
		if err := Validate(context.Background(), call, "whatsapp", time.Second); err == nil {
			t.Fatalf("unsafe pool: %s", raw)
		}
	}
}

func TestCancellationAfterPreparationStopsDatabaseMutation(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	mutated := false
	call := func(_ context.Context, _ string, req any) (json.RawMessage, error) {
		query := req.(map[string]any)["sql"].(string)
		if strings.HasPrefix(query, "PRAGMA ") {
			cancel()
			return json.RawMessage(`{"affected_rows":0}`), nil
		}
		mutated = true
		return json.RawMessage(`{"affected_rows":1}`), nil
	}
	c := &connection{connector: &connector{call: call, db: "whatsapp", prefix: "wa_test_", timeout: time.Second}}
	_, err := c.ExecContext(ctx, "DELETE FROM inbox_messages", nil)
	if mutated || !errors.Is(err, context.Canceled) {
		t.Fatalf("canceled preparation dispatched mutation=%v and returned %v", mutated, err)
	}
}

func TestTransactionLeaseUsesCallerDeadline(t *testing.T) {
	var lease int64
	call := func(_ context.Context, id string, req any) (json.RawMessage, error) {
		switch id {
		case "database::beginTransaction":
			lease = req.(map[string]any)["timeout_ms"].(int64)
			return json.RawMessage(`{"transaction":{"id":"deadline-tx"}}`), nil
		case "database::rollbackTransaction":
			return json.RawMessage(`{"rolled_back":true}`), nil
		default:
			return json.RawMessage(`{"affected_rows":0}`), nil
		}
	}
	db := Open(call, "whatsapp", "wa_test_", 5*time.Second)
	defer db.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 100*time.Millisecond)
	defer cancel()
	tx, err := db.BeginTx(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}
	_ = tx.Rollback()
	if lease < 1 || lease > 100 {
		t.Fatalf("100ms operation opened a remote transaction lease of %dms", lease)
	}
}

func TestCanceledTransactionRollbackHasBoundedCleanup(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	cleanup := make(chan context.Context, 1)
	call := func(callCtx context.Context, id string, req any) (json.RawMessage, error) {
		if p, ok := req.(map[string]any); ok && strings.HasPrefix(strings.TrimSpace(fmt.Sprint(p["sql"])), "PRAGMA ") {
			return json.RawMessage(`{"affected_rows":0}`), nil
		}
		switch id {
		case "database::beginTransaction":
			return json.RawMessage(`{"transaction":{"id":"canceled-tx"}}`), nil
		case "database::transactionExecute":
			cancel()
			return nil, ctx.Err()
		case "database::rollbackTransaction":
			cleanup <- callCtx
			return json.RawMessage(`{"rolled_back":true}`), nil
		default:
			return nil, errors.New("unexpected operation")
		}
	}
	db := Open(call, "whatsapp", "wa_test_", 5*time.Second)
	defer db.Close()
	tx, err := db.BeginTx(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}
	_, err = tx.ExecContext(ctx, "DELETE FROM inbox_messages")
	if !errors.Is(err, context.Canceled) {
		t.Fatalf("canceled mutation returned %v", err)
	}
	_ = tx.Rollback()
	select {
	case callCtx := <-cleanup:
		deadline, ok := callCtx.Deadline()
		if !ok || time.Until(deadline) > 1050*time.Millisecond {
			t.Fatal("rollback cleanup retained the five-second storage timeout")
		}
	case <-time.After(time.Second):
		t.Fatal("canceled transaction was not rolled back")
	}
}
