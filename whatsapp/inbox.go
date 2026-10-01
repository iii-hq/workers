package main

import (
	"context"
	"database/sql"
	"encoding/base64"
	"encoding/json"
	"errors"
	"math"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"time"
	"unicode/utf8"

	_ "modernc.org/sqlite"
)

type MessageView struct {
	Sequence    int64   `json:"sequence"`
	MessageID   string  `json:"message_id"`
	ChatID      string  `json:"chat_id"`
	SenderID    string  `json:"sender_id"`
	SenderName  *string `json:"sender_name"`
	FromMe      bool    `json:"from_me"`
	TimestampMS int64   `json:"timestamp_ms"`
	Kind        string  `json:"kind"`
	Text        string  `json:"text"`
	Source      string  `json:"source"`
	Status      *string `json:"status"`
	ExpiresAtMS *int64  `json:"-"`
	Edited      bool    `json:"-"`
}
type ExternalReceipt struct {
	MessageID, ChatID, Status string
	SentAtMS                  *int64
}
type Coverage struct {
	CaptureStartedAtMS      int64  `json:"capture_started_at_ms"`
	LastHistorySyncAtMS     *int64 `json:"last_history_sync_at_ms"`
	LastHistorySyncProgress *int   `json:"last_history_sync_progress"`
	MessageLimit            int    `json:"message_limit"`
	RetentionTruncated      bool   `json:"retention_truncated"`
}
type MessagePage struct {
	Messages   []MessageView `json:"messages"`
	NextCursor *string       `json:"next_cursor"`
	Coverage   Coverage      `json:"coverage"`
}
type ChatView struct {
	ChatID       string       `json:"chat_id"`
	Title        string       `json:"title"`
	IsGroup      bool         `json:"is_group"`
	LastMessage  *MessageView `json:"last_message"`
	MessageCount int          `json:"message_count"`
}
type ChatPage struct {
	Chats      []ChatView `json:"chats"`
	NextCursor *string    `json:"next_cursor"`
	Coverage   Coverage   `json:"coverage"`
}
type Inbox struct {
	mu         sync.Mutex
	db         *sql.DB
	limit      int
	generation uint64
}
type InboxCapture struct {
	inbox      *Inbox
	generation uint64
}
type inboxCursor struct {
	Scope       string `json:"scope"`
	Query       string `json:"query"`
	TimestampMS int64  `json:"timestamp_ms"`
	Key         string `json:"key"`
}

const inboxSchema = `PRAGMA busy_timeout=5000; PRAGMA secure_delete=ON; PRAGMA journal_mode=DELETE;
CREATE TABLE IF NOT EXISTS inbox_meta (id INTEGER PRIMARY KEY CHECK(id=1),capture_started_at_ms BIGINT NOT NULL,last_history_sync_at_ms BIGINT,last_history_sync_progress INTEGER,retention_truncated INTEGER NOT NULL DEFAULT 0,account TEXT);
CREATE TABLE IF NOT EXISTS inbox_chats (chat_id TEXT PRIMARY KEY,title TEXT NOT NULL,is_group INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS inbox_messages (row_id INTEGER PRIMARY KEY AUTOINCREMENT,chat_id TEXT NOT NULL,sender_key TEXT NOT NULL,message_id TEXT NOT NULL,sender_id TEXT NOT NULL,sender_name TEXT,from_me INTEGER NOT NULL,timestamp_ms BIGINT NOT NULL,kind TEXT NOT NULL,text TEXT NOT NULL,source TEXT NOT NULL,status TEXT,expires_at_ms BIGINT,edited INTEGER NOT NULL DEFAULT 0,UNIQUE(chat_id,sender_key,message_id));
CREATE INDEX IF NOT EXISTS inbox_chat_order ON inbox_messages(chat_id,timestamp_ms DESC,row_id DESC);
CREATE INDEX IF NOT EXISTS inbox_retention_order ON inbox_messages(timestamp_ms DESC,row_id DESC);
CREATE INDEX IF NOT EXISTS inbox_expiration ON inbox_messages(expires_at_ms) WHERE expires_at_ms IS NOT NULL;
CREATE TABLE IF NOT EXISTS inbox_external_receipts(message_id TEXT PRIMARY KEY,chat_id TEXT NOT NULL,status TEXT NOT NULL,rank INTEGER NOT NULL,received_at_ms BIGINT NOT NULL,sent_at_ms BIGINT);
CREATE TABLE IF NOT EXISTS inbox_cloud_event_markers(sequence INTEGER PRIMARY KEY AUTOINCREMENT,event_key TEXT UNIQUE NOT NULL);`

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
	if _, err = db.Exec(inboxSchema); err != nil {
		_ = db.Close()
		return nil, err
	}
	var hasSentTime bool
	if err = db.QueryRow("SELECT EXISTS(SELECT 1 FROM pragma_table_info('inbox_external_receipts') WHERE name='sent_at_ms')").Scan(&hasSentTime); err == nil && !hasSentTime {
		_, err = db.Exec("ALTER TABLE inbox_external_receipts ADD COLUMN sent_at_ms BIGINT")
	}
	if err != nil {
		_ = db.Close()
		return nil, err
	}
	if _, err = db.Exec("INSERT OR IGNORE INTO inbox_meta(id,capture_started_at_ms) VALUES(1,?)", time.Now().UnixMilli()); err != nil {
		_ = db.Close()
		return nil, err
	}
	i := &Inbox{db: db, limit: limit}
	if err = i.SetLimit(limit); err != nil {
		_ = db.Close()
		return nil, err
	}
	return i, nil
}

func privateInboxDirectory(dir string) error {
	abs, err := filepath.Abs(dir)
	if err != nil {
		return err
	}
	current := string(filepath.Separator)
	for _, part := range strings.Split(strings.TrimPrefix(abs, current), string(filepath.Separator)) {
		current = filepath.Join(current, part)
		info, err := os.Lstat(current)
		if err == nil {
			if info.Mode()&os.ModeSymlink != 0 || !info.IsDir() {
				return errors.New("inbox directory must not contain symlinks or non-directories")
			}
		} else if !os.IsNotExist(err) {
			return err
		}
	}
	if err = os.MkdirAll(abs, 0700); err != nil {
		return err
	}
	return os.Chmod(abs, 0700)
}

func canonicalInboxChat(raw string) (string, error) {
	if strings.HasPrefix(raw, "cloud:") {
		id, err := normalizeCloudChatID(strings.TrimPrefix(raw, "cloud:"))
		if err != nil {
			return "", err
		}
		return "cloud:" + id, nil
	}
	user, server, ok := strings.Cut(raw, "@")
	if !ok || user == "" || len(raw) > 256 {
		return "", errors.New("invalid inbox chat_id")
	}
	if server == "c.us" {
		server = "s.whatsapp.net"
	}
	if server != "s.whatsapp.net" && server != "lid" && server != "g.us" {
		return "", errors.New("invalid inbox chat_id")
	}
	user, _, _ = strings.Cut(user, ":")
	for _, c := range user {
		if (c < '0' || c > '9') && (server != "g.us" || c != '-') {
			return "", errors.New("invalid inbox chat_id")
		}
	}
	if user == "" {
		return "", errors.New("invalid inbox chat_id")
	}
	return user + "@" + server, nil
}
func clipInboxText(text string, limit int) string {
	if len(text) <= limit {
		return text
	}
	for limit > 0 && !utf8.RuneStart(text[limit]) {
		limit--
	}
	return text[:limit]
}
func (i *Inbox) active() error {
	if i.db == nil {
		return errors.New("inbox is closed")
	}
	return nil
}
func (i *Inbox) Capture() *InboxCapture {
	i.mu.Lock()
	defer i.mu.Unlock()
	return &InboxCapture{i, i.generation}
}
func (i *Inbox) Invalidate() { i.mu.Lock(); defer i.mu.Unlock(); i.generation++ }
func (i *Inbox) Close() error {
	i.mu.Lock()
	defer i.mu.Unlock()
	i.generation++
	if i.db == nil {
		return nil
	}
	err := i.db.Close()
	i.db = nil
	return err
}
func (i *Inbox) transaction(fn func(*sql.Tx) error) error {
	if err := i.active(); err != nil {
		return err
	}
	tx, err := i.db.BeginTx(context.Background(), nil)
	if err != nil {
		return err
	}
	defer tx.Rollback()
	if err = fn(tx); err != nil {
		return err
	}
	return tx.Commit()
}
func (i *Inbox) SetLimit(limit int) error {
	i.mu.Lock()
	defer i.mu.Unlock()
	if limit < 1 || limit > 100000 {
		return errors.New("max_inbox_messages must be 1..100000")
	}
	if err := i.transaction(func(tx *sql.Tx) error { return trimInbox(tx, limit) }); err != nil {
		return err
	}
	i.limit = limit
	return nil
}
func clearInbox(tx *sql.Tx) error {
	if _, err := tx.Exec("DELETE FROM inbox_messages; DELETE FROM inbox_chats; DELETE FROM inbox_external_receipts; DELETE FROM inbox_cloud_event_markers;"); err != nil {
		return err
	}
	_, err := tx.Exec("UPDATE inbox_meta SET capture_started_at_ms=?,last_history_sync_at_ms=NULL,last_history_sync_progress=NULL,retention_truncated=0,account=NULL WHERE id=1", time.Now().UnixMilli())
	return err
}
func (i *Inbox) Clear() error {
	i.mu.Lock()
	defer i.mu.Unlock()
	i.generation++
	return i.transaction(clearInbox)
}
func (i *Inbox) SetAccount(account string) error {
	i.mu.Lock()
	defer i.mu.Unlock()
	return i.transaction(func(tx *sql.Tx) error {
		var prior sql.NullString
		if err := tx.QueryRow("SELECT account FROM inbox_meta WHERE id=1").Scan(&prior); err != nil {
			return err
		}
		if prior.Valid && prior.String != account {
			i.generation++
			if err := clearInbox(tx); err != nil {
				return err
			}
		}
		_, err := tx.Exec("UPDATE inbox_meta SET account=? WHERE id=1", account)
		return err
	})
}

func expireInbox(exec interface {
	Exec(string, ...any) (sql.Result, error)
}) error {
	_, err := exec.Exec("UPDATE inbox_messages SET text='',kind='expired' WHERE expires_at_ms<=? AND kind!='expired'", time.Now().UnixMilli())
	return err
}
func trimInbox(tx *sql.Tx, limit int) error {
	if err := expireInbox(tx); err != nil {
		return err
	}
	result, err := tx.Exec("DELETE FROM inbox_messages WHERE row_id IN (SELECT row_id FROM inbox_messages ORDER BY timestamp_ms DESC,row_id DESC LIMIT -1 OFFSET ?)", limit)
	if err != nil {
		return err
	}
	removed, _ := result.RowsAffected()
	if removed > 0 {
		if _, err = tx.Exec("UPDATE inbox_meta SET retention_truncated=1 WHERE id=1"); err != nil {
			return err
		}
	}
	if _, err = tx.Exec("DELETE FROM inbox_chats WHERE chat_id NOT IN (SELECT chat_id FROM inbox_messages) AND chat_id IN (SELECT chat_id FROM inbox_chats ORDER BY chat_id LIMIT -1 OFFSET ?)", limit); err != nil {
		return err
	}
	if _, err = tx.Exec("DELETE FROM inbox_external_receipts WHERE message_id IN (SELECT message_id FROM inbox_external_receipts ORDER BY received_at_ms DESC,message_id DESC LIMIT -1 OFFSET ?)", limit); err != nil {
		return err
	}
	_, err = tx.Exec("DELETE FROM inbox_cloud_event_markers WHERE sequence IN (SELECT sequence FROM inbox_cloud_event_markers ORDER BY sequence DESC LIMIT -1 OFFSET ?)", 5*limit)
	return err
}
func validateInboxMessages(messages []MessageView) ([]MessageView, error) {
	if len(messages) > 10000 {
		return nil, errors.New("inbox batch exceeds limit")
	}
	out := make([]MessageView, len(messages))
	for n, m := range messages {
		chat, err := canonicalInboxChat(m.ChatID)
		if err != nil {
			return nil, err
		}
		if m.MessageID == "" || len(m.MessageID) > 512 || m.TimestampMS < 0 || len(m.SenderID) > 512 {
			return nil, errors.New("invalid inbox message metadata")
		}
		m.ChatID = chat
		m.Text = clipInboxText(m.Text, 16384)
		m.Kind = clipInboxText(m.Kind, 64)
		if m.SenderName != nil {
			name := clipInboxText(*m.SenderName, 512)
			m.SenderName = &name
		}
		if m.Kind == "view_once" || m.Kind == "deleted" || m.Kind == "expired" {
			m.Text = ""
		}
		out[n] = m
	}
	return out, nil
}
func commitInbox(tx *sql.Tx, messages []MessageView, titles map[string]string) error {
	for id, title := range titles {
		chat, err := canonicalInboxChat(id)
		if err != nil {
			return err
		}
		if _, err = tx.Exec("INSERT INTO inbox_chats(chat_id,title,is_group) VALUES(?,?,?) ON CONFLICT(chat_id) DO UPDATE SET title=CASE WHEN excluded.title!='' THEN excluded.title ELSE inbox_chats.title END", chat, clipInboxText(title, 512), strings.HasSuffix(chat, "@g.us")); err != nil {
			return err
		}
	}
	for _, m := range messages {
		title := m.ChatID
		if !m.FromMe && !strings.HasSuffix(m.ChatID, "@g.us") && m.SenderName != nil {
			title = *m.SenderName
		}
		if _, err := tx.Exec("INSERT INTO inbox_chats(chat_id,title,is_group) VALUES(?,?,?) ON CONFLICT(chat_id) DO UPDATE SET title=CASE WHEN inbox_chats.title=inbox_chats.chat_id THEN excluded.title ELSE inbox_chats.title END", m.ChatID, title, strings.HasSuffix(m.ChatID, "@g.us")); err != nil {
			return err
		}
		senderKey := m.SenderID
		if m.FromMe {
			senderKey = "me"
		}
		_, err := tx.Exec(`INSERT INTO inbox_messages(chat_id,sender_key,message_id,sender_id,sender_name,from_me,timestamp_ms,kind,text,source,status,expires_at_ms,edited) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)
ON CONFLICT(chat_id,sender_key,message_id) DO UPDATE SET
sender_name=COALESCE(excluded.sender_name,inbox_messages.sender_name),
kind=CASE WHEN inbox_messages.kind IN ('view_once','expired','deleted') THEN inbox_messages.kind WHEN inbox_messages.edited=1 AND excluded.edited=0 AND excluded.kind!='deleted' THEN inbox_messages.kind ELSE excluded.kind END,
text=CASE WHEN inbox_messages.kind IN ('view_once','expired','deleted') OR excluded.kind IN ('view_once','deleted','expired') THEN '' WHEN inbox_messages.edited=1 AND excluded.edited=0 THEN inbox_messages.text ELSE excluded.text END,
expires_at_ms=COALESCE(inbox_messages.expires_at_ms,excluded.expires_at_ms),edited=MAX(inbox_messages.edited,excluded.edited)`, m.ChatID, senderKey, m.MessageID, m.SenderID, m.SenderName, m.FromMe, m.TimestampMS, m.Kind, m.Text, m.Source, m.Status, m.ExpiresAtMS, m.Edited)
		if err != nil {
			return err
		}
	}
	return nil
}
func (i *Inbox) Record(messages []MessageView) error {
	prepared, err := validateInboxMessages(messages)
	if err != nil {
		return err
	}
	i.mu.Lock()
	defer i.mu.Unlock()
	return i.transaction(func(tx *sql.Tx) error {
		if err := commitInbox(tx, prepared, nil); err != nil {
			return err
		}
		return trimInbox(tx, i.limit)
	})
}
func (c *InboxCapture) Record(messages []MessageView) error {
	return c.RecordHistory(messages, nil, nil)
}
func (c *InboxCapture) RecordHistory(messages []MessageView, titles map[string]string, progress *int) error {
	prepared, err := validateInboxMessages(messages)
	if err != nil {
		return err
	}
	i := c.inbox
	i.mu.Lock()
	defer i.mu.Unlock()
	if i.generation != c.generation {
		return errors.New("inbox capture was invalidated")
	}
	return i.transaction(func(tx *sql.Tx) error {
		if err := commitInbox(tx, prepared, titles); err != nil {
			return err
		}
		if titles != nil || progress != nil {
			if progress != nil {
				p := max(0, min(100, *progress))
				progress = &p
			}
			if _, err := tx.Exec("UPDATE inbox_meta SET last_history_sync_at_ms=?,last_history_sync_progress=? WHERE id=1", time.Now().UnixMilli(), progress); err != nil {
				return err
			}
		}
		return trimInbox(tx, i.limit)
	})
}
func (i *Inbox) RecordReceipt(chatID string, ids []string, status string) error {
	i.mu.Lock()
	defer i.mu.Unlock()
	return i.recordReceipt(chatID, ids, status)
}
func (c *InboxCapture) RecordReceipt(chatID string, ids []string, status string) error {
	i := c.inbox
	i.mu.Lock()
	defer i.mu.Unlock()
	if i.generation != c.generation {
		return errors.New("inbox capture was invalidated")
	}
	return i.recordReceipt(chatID, ids, status)
}

// Caller holds the inbox mutex, including generation validation for captures.
func (i *Inbox) recordReceipt(chatID string, ids []string, status string) error {
	chat, err := canonicalInboxChat(chatID)
	if err != nil {
		return err
	}
	if strings.HasSuffix(chat, "@g.us") {
		return nil
	}
	rank := map[string]int{"sent": 1, "delivered": 2, "read": 3, "played": 4}[status]
	if rank == 0 {
		return errors.New("invalid delivery status")
	}
	return i.transaction(func(tx *sql.Tx) error {
		for _, id := range ids {
			if _, err := tx.Exec("UPDATE inbox_messages SET status=? WHERE chat_id=? AND message_id=? AND from_me=1 AND CASE status WHEN 'played' THEN 4 WHEN 'read' THEN 3 WHEN 'delivered' THEN 2 ELSE 1 END<?", status, chat, id, rank); err != nil {
				return err
			}
		}
		return nil
	})
}
func externalStatusRank(status string) int {
	return map[string]int{"sent": 1, "failed": 2, "delivered": 3, "read": 4}[status]
}
func (i *Inbox) RecordExternal(messages []MessageView, receipts []ExternalReceipt) (int, error) {
	if _, err := i.recordExternal(messages, receipts, nil); err != nil {
		return 0, err
	}
	return len(messages) + len(receipts), nil
}

// RecordCloudDelivery atomically archives a webhook and claims its unseen events.
// Markers suppress retries across restarts; they do not guarantee trigger delivery.
func (i *Inbox) RecordCloudDelivery(messages []MessageView, receipts []ExternalReceipt, events []CloudEvent) ([]CloudEvent, error) {
	return i.recordExternal(messages, receipts, events)
}

func (i *Inbox) recordExternal(messages []MessageView, receipts []ExternalReceipt, events []CloudEvent) ([]CloudEvent, error) {
	if len(messages)+len(receipts) > 1000 {
		return nil, errors.New("Cloud API update batch exceeds 1000 items")
	}
	if len(events) > 1000 {
		return nil, errors.New("Cloud API event batch exceeds 1000 items")
	}
	prepared, err := validateInboxMessages(messages)
	if err != nil {
		return nil, err
	}
	for n := range prepared {
		if !strings.HasPrefix(prepared[n].ChatID, "cloud:") {
			return nil, errors.New("invalid Cloud API chat identifier")
		}
		prepared[n].Source = "live"
		if prepared[n].FromMe {
			prepared[n].Source = "sent"
		}
	}
	for _, receipt := range receipts {
		if !strings.HasPrefix(receipt.ChatID, "cloud:") || receipt.MessageID == "" || len(receipt.MessageID) > 512 || externalStatusRank(receipt.Status) == 0 {
			return nil, errors.New("invalid Cloud API receipt")
		}
		if _, err := canonicalInboxChat(receipt.ChatID); err != nil {
			return nil, err
		}
		if receipt.SentAtMS != nil && (receipt.Status != "sent" || *receipt.SentAtMS < 0) {
			return nil, errors.New("invalid Cloud API sent timestamp")
		}
	}
	i.mu.Lock()
	defer i.mu.Unlock()
	fresh := []CloudEvent{}
	err = i.transaction(func(tx *sql.Tx) error {
		if err := commitInbox(tx, prepared, nil); err != nil {
			return err
		}
		for _, r := range receipts {
			if _, err := tx.Exec(`INSERT INTO inbox_external_receipts(message_id,chat_id,status,rank,received_at_ms,sent_at_ms) VALUES(?,?,?,?,?,?)
ON CONFLICT(message_id) DO UPDATE SET
status=CASE WHEN excluded.rank>inbox_external_receipts.rank THEN excluded.status ELSE inbox_external_receipts.status END,
received_at_ms=CASE WHEN excluded.rank>inbox_external_receipts.rank THEN excluded.received_at_ms ELSE inbox_external_receipts.received_at_ms END,
rank=MAX(excluded.rank,inbox_external_receipts.rank),
sent_at_ms=COALESCE(inbox_external_receipts.sent_at_ms,excluded.sent_at_ms)`, r.MessageID, r.ChatID, r.Status, externalStatusRank(r.Status), time.Now().UnixMilli(), r.SentAtMS); err != nil {
				return err
			}
		}
		if _, err := tx.Exec("UPDATE inbox_messages SET status=(SELECT status FROM inbox_external_receipts r WHERE r.message_id=inbox_messages.message_id) WHERE from_me=1 AND EXISTS(SELECT 1 FROM inbox_external_receipts r WHERE r.message_id=inbox_messages.message_id AND r.rank>CASE inbox_messages.status WHEN 'sent' THEN 1 WHEN 'failed' THEN 2 WHEN 'delivered' THEN 3 WHEN 'read' THEN 4 ELSE 0 END)"); err != nil {
			return err
		}
		if _, err := tx.Exec("UPDATE inbox_messages SET timestamp_ms=(SELECT sent_at_ms FROM inbox_external_receipts r WHERE r.message_id=inbox_messages.message_id) WHERE from_me=1 AND EXISTS(SELECT 1 FROM inbox_external_receipts r WHERE r.message_id=inbox_messages.message_id AND r.sent_at_ms IS NOT NULL AND r.sent_at_ms!=inbox_messages.timestamp_ms)"); err != nil {
			return err
		}
		for _, event := range events {
			key, _ := json.Marshal([]string{event.BusinessAccountID, event.PhoneNumberID, event.Kind, event.MessageID, event.Status})
			result, err := tx.Exec("INSERT OR IGNORE INTO inbox_cloud_event_markers(event_key) VALUES(?)", string(key))
			if err != nil {
				return err
			}
			inserted, err := result.RowsAffected()
			if err != nil {
				return err
			}
			if inserted == 1 {
				fresh = append(fresh, event)
			}
		}
		return trimInbox(tx, i.limit)
	})
	if err != nil {
		return nil, err
	}
	return fresh, nil
}

func decodeInboxCursor(encoded, scope, query string) (inboxCursor, error) {
	c := inboxCursor{Scope: scope, Query: query, TimestampMS: math.MaxInt64, Key: "\U0010ffff"}
	if encoded == "" {
		return c, nil
	}
	if len(encoded) > 2048 {
		return c, errors.New("invalid cursor")
	}
	raw, err := base64.RawURLEncoding.DecodeString(encoded)
	if err != nil {
		return c, errors.New("invalid cursor")
	}
	if err = json.Unmarshal(raw, &c); err != nil {
		return c, errors.New("invalid cursor")
	}
	if c.Scope != scope || c.Query != query {
		return c, errors.New("cursor belongs to another chat or search")
	}
	return c, nil
}
func encodeInboxCursor(scope, query string, timestamp int64, key string) *string {
	raw, _ := json.Marshal(inboxCursor{scope, query, timestamp, key})
	encoded := base64.RawURLEncoding.EncodeToString(raw)
	return &encoded
}
func (i *Inbox) coverage() (Coverage, error) {
	c := Coverage{MessageLimit: i.limit}
	var syncTime, progress sql.NullInt64
	err := i.db.QueryRow("SELECT capture_started_at_ms,last_history_sync_at_ms,last_history_sync_progress,retention_truncated FROM inbox_meta WHERE id=1").Scan(&c.CaptureStartedAtMS, &syncTime, &progress, &c.RetentionTruncated)
	if syncTime.Valid {
		c.LastHistorySyncAtMS = &syncTime.Int64
	}
	if progress.Valid {
		p := int(progress.Int64)
		c.LastHistorySyncProgress = &p
	}
	return c, err
}

const inboxMessageColumns = "row_id,message_id,chat_id,sender_id,sender_name,from_me,timestamp_ms,kind,text,source,status,expires_at_ms,edited"

func scanInboxMessage(row interface{ Scan(...any) error }) (MessageView, int64, error) {
	var m MessageView
	var id int64
	err := row.Scan(&id, &m.MessageID, &m.ChatID, &m.SenderID, &m.SenderName, &m.FromMe, &m.TimestampMS, &m.Kind, &m.Text, &m.Source, &m.Status, &m.ExpiresAtMS, &m.Edited)
	m.Sequence = id
	return m, id, err
}
func (i *Inbox) ListMessages(chatID, cursor string, limit int) (MessagePage, error) {
	page := MessagePage{Messages: []MessageView{}}
	if limit < 1 || limit > 200 {
		return page, errors.New("limit must be 1..200")
	}
	chat, err := canonicalInboxChat(chatID)
	if err != nil {
		return page, err
	}
	c, err := decodeInboxCursor(cursor, chat, "")
	if err != nil {
		return page, err
	}
	beforeID := int64(math.MaxInt64)
	if cursor != "" {
		beforeID, err = strconv.ParseInt(c.Key, 10, 64)
		if err != nil {
			return page, errors.New("invalid cursor")
		}
	}
	i.mu.Lock()
	defer i.mu.Unlock()
	if err = i.active(); err != nil {
		return page, err
	}
	if err = expireInbox(i.db); err != nil {
		return page, err
	}
	rows, err := i.db.Query("SELECT "+inboxMessageColumns+" FROM inbox_messages WHERE chat_id=? AND (timestamp_ms<? OR (timestamp_ms=? AND row_id<?)) ORDER BY timestamp_ms DESC,row_id DESC LIMIT ?", chat, c.TimestampMS, c.TimestampMS, beforeID, limit+1)
	if err != nil {
		return page, err
	}
	var lastID int64
	more := false
	for rows.Next() {
		m, id, e := scanInboxMessage(rows)
		if e != nil {
			_ = rows.Close()
			return page, e
		}
		if len(page.Messages) == limit {
			more = true
			break
		}
		page.Messages = append(page.Messages, m)
		lastID = id
	}
	err = rows.Err()
	_ = rows.Close()
	if err != nil {
		return page, err
	}
	if more {
		last := page.Messages[len(page.Messages)-1]
		page.NextCursor = encodeInboxCursor(chat, "", last.TimestampMS, strconv.FormatInt(lastID, 10))
	}
	page.Coverage, err = i.coverage()
	return page, err
}
func (i *Inbox) ListChats(query, cursor string, limit int) (ChatPage, error) {
	page := ChatPage{Chats: []ChatView{}}
	query = strings.TrimSpace(query)
	if limit < 1 || limit > 200 {
		return page, errors.New("limit must be 1..200")
	}
	if len(query) > 256 {
		return page, errors.New("query must be at most 256 bytes")
	}
	c, err := decodeInboxCursor(cursor, "chats", query)
	if err != nil {
		return page, err
	}
	i.mu.Lock()
	defer i.mu.Unlock()
	if err = i.active(); err != nil {
		return page, err
	}
	if err = expireInbox(i.db); err != nil {
		return page, err
	}
	rows, err := i.db.Query(`SELECT c.chat_id,c.title,c.is_group,COALESCE(MAX(m.timestamp_ms),0),COUNT(m.row_id) FROM inbox_chats c LEFT JOIN inbox_messages m ON m.chat_id=c.chat_id
WHERE (?='' OR instr(lower(c.title),lower(?))>0 OR instr(c.chat_id,?)>0 OR EXISTS(SELECT 1 FROM inbox_messages search WHERE search.chat_id=c.chat_id AND instr(lower(search.text),lower(?))>0))
GROUP BY c.chat_id HAVING COALESCE(MAX(m.timestamp_ms),0)<? OR (COALESCE(MAX(m.timestamp_ms),0)=? AND c.chat_id<?) ORDER BY COALESCE(MAX(m.timestamp_ms),0) DESC,c.chat_id DESC LIMIT ?`, query, query, query, query, c.TimestampMS, c.TimestampMS, c.Key, limit+1)
	if err != nil {
		return page, err
	}
	var lastTime int64
	more := false
	for rows.Next() {
		var chat ChatView
		var timestamp int64
		if err = rows.Scan(&chat.ChatID, &chat.Title, &chat.IsGroup, &timestamp, &chat.MessageCount); err != nil {
			_ = rows.Close()
			return page, err
		}
		if len(page.Chats) == limit {
			more = true
			break
		}
		page.Chats = append(page.Chats, chat)
		lastTime = timestamp
	}
	err = rows.Err()
	_ = rows.Close()
	if err != nil {
		return page, err
	}
	if more {
		page.NextCursor = encodeInboxCursor("chats", query, lastTime, page.Chats[len(page.Chats)-1].ChatID)
	}
	for n := range page.Chats {
		m, _, e := scanInboxMessage(i.db.QueryRow("SELECT "+inboxMessageColumns+" FROM inbox_messages WHERE chat_id=? ORDER BY timestamp_ms DESC,row_id DESC LIMIT 1", page.Chats[n].ChatID))
		if e == nil {
			page.Chats[n].LastMessage = &m
		} else if !errors.Is(e, sql.ErrNoRows) {
			return page, e
		}
	}
	page.Coverage, err = i.coverage()
	return page, err
}
func (i *Inbox) Oldest(chatID string) (MessageView, error) {
	chat, err := canonicalInboxChat(chatID)
	if err != nil {
		return MessageView{}, err
	}
	i.mu.Lock()
	defer i.mu.Unlock()
	if err = i.active(); err != nil {
		return MessageView{}, err
	}
	if err = expireInbox(i.db); err != nil {
		return MessageView{}, err
	}
	m, _, err := scanInboxMessage(i.db.QueryRow("SELECT "+inboxMessageColumns+" FROM inbox_messages WHERE chat_id=? AND timestamp_ms>0 ORDER BY timestamp_ms,row_id LIMIT 1", chat))
	if errors.Is(err, sql.ErrNoRows) {
		return m, errors.New("history requires a known message in this chat; wait for a message or received sync")
	}
	return m, err
}
