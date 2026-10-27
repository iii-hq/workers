package main

import (
	"context"
	"encoding/json"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"go.mau.fi/whatsmeow/proto/waCompanionReg"
	"go.mau.fi/whatsmeow/store"
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
	changed.DeviceName = "Example Desk"
	changed.DevicePlatform = "chrome"
	changed.MaxInboxMessages = 2
	changed.Cloud.AccessToken = "private-sentinel"
	got, pending, err := m.applyConfigSnapshotContext(context.Background(), changed)
	if err != nil || !reflect.DeepEqual(pending, []string{"data_dir", "device_name", "device_platform", "max_events"}) {
		t.Fatalf("reload %v %v", pending, err)
	}
	if got.DataDir != c.DataDir || got.DeviceName != c.DeviceName || got.DevicePlatform != c.DevicePlatform || got.MaxEvents != c.MaxEvents || got.MaxInboxMessages != 2 || got.Cloud.AccessToken != "private-sentinel" {
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
func TestConfigDeviceNameAndPlatform(t *testing.T) {
	clearCloudEnvironment(t)
	c := DefaultConfig()
	if c.DeviceName != "" || c.DevicePlatform != "desktop" {
		t.Fatalf("device defaults %q %q", c.DeviceName, c.DevicePlatform)
	}
	for _, tc := range []struct {
		name, platform string
		ok             bool
	}{{"Example Desk", "chrome", true}, {strings.Repeat("é", 64), "unknown", true}, {strings.Repeat("é", 65), "desktop", false}, {"Example\nDesk", "desktop", false}, {"Example", "DESKTOP", false}, {"Example", "nope", false}, {"Example", "deſktop", false}} {
		c.DeviceName, c.DevicePlatform = tc.name, tc.platform
		if err := c.Validate(); (err == nil) != tc.ok {
			t.Errorf("validate %q %q: %v", tc.name, tc.platform, err)
		}
		b, _ := json.Marshal(c)
		if _, err := parseConfigJSON(b); (err == nil) != tc.ok {
			t.Errorf("parse %q %q: %v", tc.name, tc.platform, err)
		}
	}
	var schema struct {
		Properties map[string]struct{ Enum []string }
	}
	if err := json.Unmarshal(configSchema, &schema); err != nil {
		t.Fatal(err)
	}
	enum := schema.Properties["device_platform"].Enum
	if len(enum) != len(waCompanionReg.DeviceProps_PlatformType_value) {
		t.Fatalf("schema lists %d platforms, whatsmeow has %d", len(enum), len(waCompanionReg.DeviceProps_PlatformType_value))
	}
	for _, p := range enum {
		if _, ok := waCompanionReg.DeviceProps_PlatformType_value[strings.ToUpper(p)]; !ok || p != strings.ToLower(p) {
			t.Errorf("schema platform %q is not a lowercase whatsmeow platform type", p)
		}
	}
}
func TestApplyDevicePropsOnlyWithAName(t *testing.T) {
	name, platform := store.DeviceProps.Os, store.DeviceProps.PlatformType
	t.Cleanup(func() { store.DeviceProps.Os, store.DeviceProps.PlatformType = name, platform })
	c := DefaultConfig()
	c.DeviceName = "  "
	applyDeviceProps(c)
	if store.DeviceProps.Os != name || store.DeviceProps.PlatformType != platform {
		t.Fatal("an empty device_name changed the whatsmeow defaults")
	}
	c.DeviceName, c.DevicePlatform = " Example Desk ", "chrome"
	applyDeviceProps(c)
	if store.DeviceProps.GetOs() != "Example Desk" || store.DeviceProps.GetPlatformType() != waCompanionReg.DeviceProps_CHROME {
		t.Fatalf("device props %q %v", store.DeviceProps.GetOs(), store.DeviceProps.GetPlatformType())
	}
}
