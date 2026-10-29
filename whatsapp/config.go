package main

import (
	"bytes"
	"context"
	_ "embed"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"os"
	"strings"
	"sync"
	"time"
	"unicode"
	"unicode/utf8"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	validate "github.com/santhosh-tekuri/jsonschema/v6"
	"go.mau.fi/whatsmeow/proto/waCompanionReg"
	"go.yaml.in/yaml/v3"
)

//go:embed config.schema.json
var configSchema json.RawMessage

var compiledConfigSchema = func() *validate.Schema {
	var schema any
	if err := json.Unmarshal(configSchema, &schema); err != nil {
		panic(err)
	}
	c := validate.NewCompiler()
	if err := c.AddResource("https://whatsapp.invalid/config.json", schema); err != nil {
		panic(err)
	}
	s, err := c.Compile("https://whatsapp.invalid/config.json")
	if err != nil {
		panic(err)
	}
	return s
}()

type DatabaseConfig struct {
	Name      string `json:"name" yaml:"name"`
	Namespace string `json:"namespace" yaml:"namespace"`
}

type Config struct {
	Database             DatabaseConfig `json:"database" yaml:"database"`
	DataDir              string         `json:"data_dir" yaml:"data_dir"`
	DeviceName           string         `json:"device_name" yaml:"device_name"`
	DevicePlatform       string         `json:"device_platform" yaml:"device_platform"`
	MaxSessions          int            `json:"max_sessions" yaml:"max_sessions"`
	MaxMediaBytes        int            `json:"max_media_bytes" yaml:"max_media_bytes"`
	MaxEvents            int            `json:"max_events" yaml:"max_events"`
	MaxInboxMessages     int            `json:"max_inbox_messages" yaml:"max_inbox_messages"`
	OperationTimeoutSecs int            `json:"operation_timeout_secs" yaml:"operation_timeout_secs"`
	ShutdownTimeoutSecs  int            `json:"shutdown_timeout_secs" yaml:"shutdown_timeout_secs"`
	Cloud                CloudConfig    `json:"cloud" yaml:"cloud"`
}

func DefaultConfig() Config {
	return Config{DataDir: "data/whatsapp-go", DevicePlatform: "desktop", MaxSessions: 32, MaxMediaBytes: 16 * 1024 * 1024, MaxEvents: 1000, MaxInboxMessages: 10000, OperationTimeoutSecs: 30, ShutdownTimeoutSecs: 15, Cloud: DefaultCloudConfig(), Database: DatabaseConfig{Name: "whatsapp"}}
}
func (c Config) Validate() error {
	if strings.TrimSpace(c.Database.Name) == "" || len(c.Database.Name) > 128 || len(c.Database.Namespace) > 256 {
		return errors.New("database.name is required (max 128 characters); database.namespace must be at most 256 characters")
	}
	if strings.TrimSpace(c.DataDir) == "" {
		return errors.New("data_dir must not be empty")
	}
	if utf8.RuneCountInString(c.DeviceName) > 64 || strings.IndexFunc(c.DeviceName, unicode.IsControl) >= 0 {
		return errors.New("device_name must be at most 64 characters without control characters")
	}
	if v, ok := waCompanionReg.DeviceProps_PlatformType_value[strings.ToUpper(c.DevicePlatform)]; !ok || c.DevicePlatform != strings.ToLower(waCompanionReg.DeviceProps_PlatformType_name[v]) {
		return errors.New("device_platform must be a lowercase WhatsApp platform type such as desktop or chrome")
	}
	for _, v := range []struct {
		name       string
		value, max int
	}{{"max_sessions", c.MaxSessions, 1024}, {"max_media_bytes", c.MaxMediaBytes, 64 * 1024 * 1024}, {"max_events", c.MaxEvents, 100000}, {"max_inbox_messages", c.MaxInboxMessages, 100000}, {"operation_timeout_secs", c.OperationTimeoutSecs, 600}, {"shutdown_timeout_secs", c.ShutdownTimeoutSecs, 120}} {
		if v.value < 1 || v.value > v.max {
			return fmt.Errorf("%s must be 1..%d", v.name, v.max)
		}
	}
	return c.Cloud.Validate()
}
func (c Config) Redacted() Config { c.Cloud = c.Cloud.Redacted(); return c }
func decodeConfigJSON(data []byte) (Config, error) {
	c := DefaultConfig()
	var value any
	if json.Unmarshal(data, &value) != nil || compiledConfigSchema.Validate(value) != nil {
		return c, errors.New("invalid WhatsApp configuration structure")
	}
	d := json.NewDecoder(bytes.NewReader(data))
	d.DisallowUnknownFields()
	if bytes.Equal(bytes.TrimSpace(data), []byte("null")) {
		return c, errors.New("missing WhatsApp configuration")
	}
	if err := d.Decode(&c); err != nil {
		return c, errors.New("invalid WhatsApp configuration structure")
	}
	var trailing any
	if err := d.Decode(&trailing); err != io.EOF {
		return c, errors.New("invalid WhatsApp configuration structure")
	}
	return c, nil
}

// Central values have already been expanded by the configuration worker. Only
// apply the named process overrides; never interpolate the received strings.
func parseConfigJSON(data []byte) (Config, error) {
	c, err := decodeConfigJSON(data)
	if err != nil {
		return c, err
	}
	return runtimeConfig(c)
}

// Validate the seed with runtime overrides, but return its original values so
// environment credentials cannot be persisted by configuration::ensure.
func parseConfigYAML(data []byte) (Config, error) {
	c := DefaultConfig()
	d := yaml.NewDecoder(bytes.NewReader(data))
	var value any
	if err := d.Decode(&value); err != nil {
		return c, errors.New("invalid WhatsApp configuration structure")
	}
	var trailing any
	if err := d.Decode(&trailing); err != io.EOF {
		return c, errors.New("invalid WhatsApp configuration structure")
	}
	encoded, err := json.Marshal(value)
	if err != nil {
		return c, errors.New("invalid WhatsApp configuration structure")
	}
	c, err = decodeConfigJSON(encoded)
	if err != nil {
		return c, err
	}
	_, err = runtimeConfig(c)
	return c, err
}
func ConfigID() string {
	if s := strings.TrimSpace(os.Getenv("III_CONFIG_NAME")); s != "" {
		return s
	}
	return "whatsapp"
}
func Namespace() string {
	if s := strings.TrimSpace(os.Getenv("III_NAMESPACE")); s != "" {
		return s
	}
	return "default"
}
func controlCall(ctx context.Context, client *iii.Client, id string, data any) (json.RawMessage, error) {
	b, err := json.Marshal(data)
	if err != nil {
		return nil, err
	}
	result, err := client.Trigger(ctx, iii.TriggerRequest{FunctionID: id, Data: b, Namespace: "default", Timeout: 15 * time.Second})
	return result, err
}
func fetchConfig(ctx context.Context, client *iii.Client) (Config, error) {
	b, err := controlCall(ctx, client, "configuration::get", map[string]any{"id": ConfigID()})
	if err != nil {
		return Config{}, errors.New("unable to fetch WhatsApp configuration")
	}
	var envelope struct {
		Value json.RawMessage `json:"value"`
	}
	if json.Unmarshal(b, &envelope) != nil || len(envelope.Value) == 0 {
		return Config{}, errors.New("missing WhatsApp configuration")
	}
	return parseConfigJSON(envelope.Value)
}
func ensureConfig(ctx context.Context, client *iii.Client, seed Config) (Config, error) {
	payload := map[string]any{"id": ConfigID(), "name": "WhatsApp", "description": "WhatsApp linked devices, official Cloud API, storage and operating limits", "schema": configSchema, "metadata": map[string]any{"ui_form": "whatsapp"}, "initial_value": seed}
	_, err := controlCall(ctx, client, "configuration::ensure", payload)
	if err != nil {
		var remote *iii.InvocationError
		if !errors.As(err, &remote) || remote.Code != "function_not_found" {
			return Config{}, errors.New("unable to initialize WhatsApp configuration")
		}
		warnLegacyConfigOnce(ConfigID())
		// Older configuration workers have no atomic ensure operation. Never replace
		// an existing value, including unresolved secret templates, with seed data.
		b, getErr := controlCall(ctx, client, "configuration::get", map[string]any{"id": ConfigID(), "raw": true})
		var current map[string]json.RawMessage
		if getErr == nil {
			if json.Unmarshal(b, &current) != nil || len(current["value"]) == 0 {
				return Config{}, errors.New("invalid configuration response")
			}
			if !bytes.Equal(bytes.TrimSpace(current["value"]), []byte("null")) {
				delete(payload, "initial_value")
			}
		} else if !errors.As(getErr, &remote) || remote.Code != "NOT_FOUND" {
			return Config{}, errors.New("unable to read existing WhatsApp configuration")
		}
		if _, err = controlCall(ctx, client, "configuration::register", payload); err != nil {
			return Config{}, errors.New("unable to register WhatsApp configuration")
		}
	}
	return fetchConfig(ctx, client)
}

var legacyConfigWarnings sync.Map

func warnLegacyConfigOnce(id string) {
	if _, warned := legacyConfigWarnings.LoadOrStore(id, struct{}{}); !warned {
		log.Printf("configuration entry %q: engine lacks configuration::ensure; using non-atomic legacy initialization; upgrade to >=0.24.1 for concurrent-write safety", id)
	}
}

type ConfigStatus struct {
	ID                    string            `json:"id"`
	Source                string            `json:"source"`
	LastOutcome           string            `json:"last_outcome"`
	LastError             *string           `json:"last_error" jsonschema:"nullable"`
	RejectedReloads       uint64            `json:"rejected_reloads"`
	RestartRequiredFields []string          `json:"restart_required_fields" jsonschema:"nullable"`
	EnvironmentOverrides  map[string]string `json:"environment_overrides"`
	Active                Config            `json:"active"`
}

func registerConfiguration(ctx context.Context, client *iii.Client, m *Manager, local bool) error {
	active, err := m.ConfigContext(ctx)
	if err != nil {
		return err
	}
	var mu sync.Mutex
	status := ConfigStatus{ID: ConfigID(), Source: "central", LastOutcome: "applied", RestartRequiredFields: []string{}, EnvironmentOverrides: cloudEnvironmentOverrides(), Active: active.Redacted()}
	if local {
		status.Source = "local"
	}
	internal := iii.RegisterFunctionOptions{Metadata: json.RawMessage(`{"internal":true,"trace_hidden":true}`)}
	if err := iii.RegisterFunctionTyped(client, "whatsapp::configuration-id", func(context.Context, callbackRequest[callbackNotification]) (callbackConfigurationID, error) {
		return callbackConfigurationID{ID: ConfigID()}, nil
	}, internal); err != nil {
		return err
	}
	if err := iii.RegisterFunctionTyped(client, "whatsapp::config-status", func(ctx context.Context, _ callbackRequest[callbackNotification]) (ConfigStatus, error) {
		if err := lockSessionContext(ctx, &mu); err != nil {
			return ConfigStatus{}, err
		}
		defer mu.Unlock()
		return status, nil
	}, internal); err != nil {
		return err
	}
	if local {
		return nil
	}
	reload := func(ctx context.Context) error {
		if err := lockSessionContext(ctx, &mu); err != nil {
			return err
		}
		defer mu.Unlock()
		cfg, err := fetchConfig(ctx, client)
		var accepted Config
		var pending []string
		if err == nil {
			accepted, pending, err = m.applyConfigSnapshotContext(ctx, cfg)
		}
		if err != nil {
			message := "WhatsApp configuration reload rejected; active settings retained"
			status.LastError = &message
			status.LastOutcome = "rejected"
			status.RejectedReloads++
			return nil
		}
		status.LastError = nil
		status.LastOutcome = "applied"
		if len(pending) > 0 {
			status.LastOutcome = "restart_required"
		}
		status.RestartRequiredFields = pending
		status.EnvironmentOverrides = cloudEnvironmentOverrides()
		status.Active = accepted.Redacted()
		return nil
	}
	if err := iii.RegisterFunctionTyped(client, "whatsapp::on-config-change", func(ctx context.Context, _ callbackRequest[callbackNotification]) (callbackOK, error) {
		if m.interfaceOnly {
			return callbackOK{}, interfaceCallbackError()
		}
		if err := reload(ctx); err != nil {
			return callbackOK{}, err
		}
		return callbackOK{OK: true}, nil
	}, internal); err != nil {
		return err
	}
	b, _ := json.Marshal(map[string]any{"configuration_id": ConfigID(), "event_types": []string{"configuration:updated"}})
	if err := client.RegisterTriggerNamespaced("whatsapp:configuration:"+ConfigID(), "configuration", "whatsapp::on-config-change", Namespace(), b); err != nil {
		return err
	}
	if !m.interfaceOnly {
		return reload(ctx)
	}
	return nil
}
