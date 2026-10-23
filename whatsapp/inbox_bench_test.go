package main

import (
	"fmt"
	"strconv"
	"testing"
)

const inboxBenchmarkTimestamp int64 = 1_700_000_000_000

// Fixtures use the production schema and a temporary on-disk database. A single
// prepared transaction seeds rows outside timing; it does not measure capture,
// history transfer, or network latency. All outgoing rows use commitInbox's "me"
// sender key, and Cloud fixtures alternate incoming and outgoing messages.
func inboxBenchmarkFixture(b *testing.B, messages, chats int, cloud bool) (*Inbox, []string) {
	b.Helper()
	inbox, err := NewInbox(b.TempDir(), messages)
	if err != nil {
		b.Fatal(err)
	}
	b.Cleanup(func() {
		if err := inbox.Close(); err != nil {
			b.Error(err)
		}
	})
	tx, err := inbox.db.Begin()
	if err != nil {
		b.Fatal(err)
	}
	defer tx.Rollback()
	chatIDs := make([]string, chats)
	for n := range chatIDs {
		chatIDs[n] = fmt.Sprintf("1555%07d@s.whatsapp.net", n)
		if cloud {
			chatIDs[n] = fmt.Sprintf("cloud:+1555%07d", n)
		}
		if _, err := tx.Exec("INSERT INTO inbox_chats(chat_id,title,is_group) VALUES(?,?,0)", chatIDs[n], fmt.Sprintf("Synthetic chat %d", n)); err != nil {
			b.Fatal(err)
		}
	}
	stmt, err := tx.Prepare(`INSERT INTO inbox_messages
(chat_id,sender_key,message_id,sender_id,from_me,timestamp_ms,kind,text,source,status)
VALUES(?,?,?,?,?,?,'text','Synthetic benchmark message',?,?)`)
	if err != nil {
		b.Fatal(err)
	}
	defer stmt.Close()
	for n := 0; n < messages; n++ {
		chatID := chatIDs[n%chats]
		fromMe := !cloud || n%2 == 0
		senderKey, source, status := "me", "sent", "sent"
		if !fromMe {
			senderKey, source, status = chatID, "live", ""
		}
		if _, err := stmt.Exec(chatID, senderKey, fmt.Sprintf("fixture-%d", n), chatID, fromMe, inboxBenchmarkTimestamp+int64(n), source, status); err != nil {
			b.Fatal(err)
		}
	}
	if err := tx.Commit(); err != nil {
		b.Fatal(err)
	}
	return inbox, chatIDs
}

func BenchmarkInboxRecordReceipt(b *testing.B) {
	for _, size := range []int{1000, 10000, 100000} {
		b.Run(fmt.Sprintf("messages=%d", size), func(b *testing.B) {
			inbox, chats := inboxBenchmarkFixture(b, size, 1, false)
			ids := []string{fmt.Sprintf("fixture-%d", size-1)}
			// Measure a replay after the initial status transition, keeping both
			// the archive and the write workload fixed across iterations.
			if err := inbox.RecordReceipt(chats[0], ids, "delivered"); err != nil {
				b.Fatal(err)
			}
			b.ReportAllocs()
			for b.Loop() {
				if err := inbox.RecordReceipt(chats[0], ids, "delivered"); err != nil {
					b.Fatal(err)
				}
			}
		})
	}
}

func BenchmarkInboxRecordCloudReplay(b *testing.B) {
	for _, size := range []int{1000, 10000, 100000} {
		b.Run(fmt.Sprintf("messages=%d", size), func(b *testing.B) {
			inbox, chats := inboxBenchmarkFixture(b, size, 100, true)
			messages := []MessageView{{
				MessageID: "fixture-1", ChatID: chats[1], SenderID: chats[1],
				TimestampMS: inboxBenchmarkTimestamp + 1, Kind: "text",
				Text: "Synthetic benchmark message", Source: "live",
			}}
			// No receipts or event markers: isolate the existing archive's
			// overhead for an incoming-only retry with no archive growth.
			b.ReportAllocs()
			for b.Loop() {
				if _, err := inbox.RecordCloudDelivery(messages, nil, nil); err != nil {
					b.Fatal(err)
				}
			}
		})
	}
}

func BenchmarkInboxListMessages(b *testing.B) {
	for _, size := range []int{10000, 100000} {
		for _, position := range []string{"latest", "oldest10percent"} {
			b.Run(fmt.Sprintf("messages=%d/%s", size, position), func(b *testing.B) {
				inbox, chats := inboxBenchmarkFixture(b, size, 1, false)
				cursor := ""
				if position != "latest" {
					row := size / 10
					cursor = *encodeInboxCursor(chats[0], "", inboxBenchmarkTimestamp+int64(row-1), strconv.Itoa(row))
				}
				b.ReportAllocs()
				for b.Loop() {
					page, err := inbox.ListMessages(chats[0], cursor, 100)
					if err != nil {
						b.Fatal(err)
					}
					if len(page.Messages) != 100 {
						b.Fatalf("expected 100 messages, got %d", len(page.Messages))
					}
				}
			})
		}
	}
}

func BenchmarkInboxListChats(b *testing.B) {
	for _, size := range []int{1000, 10000, 100000} {
		b.Run(fmt.Sprintf("messages=%d", size), func(b *testing.B) {
			inbox, _ := inboxBenchmarkFixture(b, size, 100, false)
			b.ReportAllocs()
			for b.Loop() {
				page, err := inbox.ListChats("", "", 100)
				if err != nil {
					b.Fatal(err)
				}
				if len(page.Chats) != 100 {
					b.Fatalf("expected 100 chats, got %d", len(page.Chats))
				}
			}
		})
	}
}
