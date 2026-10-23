package main

import (
	"context"
	"encoding/json"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

func TestConfigDefaultsAndStrictSecretSafeParsing(t *testing.T) {
	clearCloudEnvironment(t)
	for _, tc := range []struct {
		name  string
		parse func([]byte) (Config, error)
		input string
	}{{"json", parseConfigJSON, `{"cloud":{"access_token":"secret"}}`}, {"yaml", parseConfigYAML, "cloud:\n  access_token: secret\n"}} {
		t.Run(tc.name, func(t *testing.T) {
			c, err := tc.parse([]byte(tc.input))
			want := DefaultConfig()
			want.Cloud.AccessToken = "secret"
			if err != nil || !reflect.DeepEqual(c, want) {
				t.Fatalf("defaults not preserved: %v", err)
			}
		})
	}
	for _, input := range []string{`null`, `{"max_sessions":null}`, `{"cloud":null}`, `{"cloud":{"unknown":"private-sentinel"}}`, `{"max_media_bytes":-1}`, `{"max_events":100001}`, `{"cloud":{"enabled":true}}`, `{"data_dir":""}`, `{} {}`} {
		_, err := parseConfigJSON([]byte(input))
		if err == nil {
			t.Errorf("invalid config accepted: %s", input)
		} else if strings.Contains(err.Error(), "private-sentinel") {
			t.Fatal("secret leaked in parse error")
		}
	}
	for _, input := range []string{"max_events: 1\nmax_events: 2", "cloud:\n  private-sentinel: token", "data_dir: null", "---\n{}\n---\n{}"} {
		if _, err := parseConfigYAML([]byte(input)); err == nil {
			t.Errorf("invalid YAML accepted: %q", input)
		}
	}
}
func TestConfigReloadKeepsStructuralValuesAndDefersInboxTrim(t *testing.T) {
	c := DefaultConfig()
	c.DataDir = filepath.Join(t.TempDir(), "sessions")
	m, err := newTestManager(c)
	if err != nil {
		t.Fatal(err)
	}
	defer m.Shutdown(context.Background())
	if _, err = m.Create("test"); err != nil {
		t.Fatal(err)
	}
	for n, id := range []string{"one", "two", "three"} {
		if err = m.RecordSent("test", MessageView{MessageID: id, ChatID: "1234567@s.whatsapp.net", SenderID: "1234567@s.whatsapp.net", TimestampMS: int64(n + 1), Kind: "text", Text: id, Source: "live"}); err != nil {
			t.Fatal(err)
		}
	}
	changed := c
	changed.DataDir = "another-directory"
	changed.MaxEvents = 23
	changed.MaxInboxMessages = 2
	changed.Cloud.AccessToken = "private-sentinel"
	got, pending, err := m.applyConfigSnapshotContext(context.Background(), changed)
	if err != nil || !reflect.DeepEqual(pending, []string{"data_dir", "max_events"}) {
		t.Fatalf("reload %v %v", pending, err)
	}
	if got.DataDir != c.DataDir || got.MaxEvents != c.MaxEvents || got.MaxInboxMessages != 2 || got.Cloud.AccessToken != "private-sentinel" {
		t.Fatal("reload state mismatch")
	}
	if !reflect.DeepEqual(got, m.Config()) {
		t.Fatal("accepted configuration snapshot differs from active settings")
	}
	b, _ := json.Marshal(got.Redacted())
	if strings.Contains(string(b), "private-sentinel") {
		t.Fatal("redacted configuration exposed a token")
	}
	box, _ := m.Inbox("test")
	page, err := box.ListMessages("1234567@s.whatsapp.net", "", 50)
	if err != nil || len(page.Messages) != 3 || page.Coverage.RetentionTruncated {
		t.Fatalf("reload must preserve inbox data until its next write: %#v %v", page, err)
	}
	if err = m.RecordSent("test", MessageView{MessageID: "four", ChatID: "1234567@s.whatsapp.net", SenderID: "1234567@s.whatsapp.net", TimestampMS: 4, Kind: "text", Text: "four", Source: "live"}); err != nil {
		t.Fatal(err)
	}
	page, err = box.ListMessages("1234567@s.whatsapp.net", "", 50)
	if err != nil || len(page.Messages) != 2 || !page.Coverage.RetentionTruncated {
		t.Fatalf("retention not applied: %#v %v", page, err)
	}
	invalid := changed
	invalid.MaxSessions = 0
	if _, err = m.ApplyConfig(invalid); err == nil {
		t.Fatal("invalid reload applied")
	}
	if !reflect.DeepEqual(m.Config(), got) {
		t.Fatal("invalid reload altered state")
	}
}
