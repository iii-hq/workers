package main

import (
	"encoding/json"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

// The configuration service deliberately has no Cloud environment variables.
// Only the child worker receives credentials, which must stay out of its seed.
func TestCloudEnvironmentEngine(t *testing.T) {
	engine, worker := os.Getenv("III_BINARY"), os.Getenv("WHATSAPP_BINARY")
	database := os.Getenv("DATABASE_BINARY")
	if engine == "" || worker == "" || database == "" {
		t.Skip("set III_BINARY, WHATSAPP_BINARY and DATABASE_BINARY for isolated loopback integration")
	}
	clearCloudEnvironment(t)
	dir := t.TempDir()
	socket, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	port := socket.Addr().(*net.TCPAddr).Port
	_ = socket.Close()
	engineURL := fmt.Sprintf("ws://127.0.0.1:%d", port)
	engineConfig := filepath.Join(dir, "engine.yaml")
	if err = os.WriteFile(engineConfig, []byte(fmt.Sprintf("workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: %d\n  - name: configuration\n    config:\n      adapter:\n        name: fs\n        config:\n          directory: ./configuration\n", port)), 0600); err != nil {
		t.Fatal(err)
	}
	smokeProcess(t, dir, "engine", engine, []string{"--config", engineConfig, "--no-update-check"}, "III_NAMESPACE=default")
	smokeWait(t, "environment test engine", func() bool {
		_, err := smokeValue(engine, engineURL, "default", "engine::workers::list", map[string]any{})
		return err == nil
	})

	environment := map[string]string{
		"WHATSAPP_CLOUD_ENABLED":             "true",
		"WHATSAPP_CLOUD_API_VERSION":         "v25.0",
		"WHATSAPP_CLOUD_BUSINESS_ACCOUNT_ID": "00012345678901234567890123456789",
		"WHATSAPP_CLOUD_PHONE_NUMBER_ID":     "98765432109876543210987654321098",
		"WHATSAPP_CLOUD_ACCESS_TOKEN":        `env-only-access: "quoted" ${UNEXPANDED_SECRET}`,
		"WHATSAPP_META_APP_SECRET":           "env-only-app-secret",
		"WHATSAPP_WEBHOOK_VERIFY_TOKEN":      "env-only-verify-token",
	}
	wantOverrides := map[string]any{
		"cloud.enabled":             "WHATSAPP_CLOUD_ENABLED",
		"cloud.api_version":         "WHATSAPP_CLOUD_API_VERSION",
		"cloud.business_account_id": "WHATSAPP_CLOUD_BUSINESS_ACCOUNT_ID",
		"cloud.phone_number_id":     "WHATSAPP_CLOUD_PHONE_NUMBER_ID",
		"cloud.access_token":        "WHATSAPP_CLOUD_ACCESS_TOKEN",
		"cloud.app_secret":          "WHATSAPP_META_APP_SECRET",
		"cloud.verify_token":        "WHATSAPP_WEBHOOK_VERIFY_TOKEN",
	}
	for _, mode := range []string{"seed", "existing", "local"} {
		t.Run(mode, func(t *testing.T) {
			namespace := "whatsapp-env-" + mode
			databaseSeed := filepath.Join(dir, namespace+"-database.yaml")
			if err := os.WriteFile(databaseSeed, []byte("databases:\n  whatsapp:\n    url: sqlite:"+filepath.Join(t.TempDir(), "whatsapp.db")+"\n    pool:\n      max: 1\nhistory_max_entries: 0\nhistory_max_bytes: 0\n"), 0600); err != nil {
				t.Fatal(err)
			}
			smokeProcess(t, dir, namespace+"-database", database, []string{"--url", engineURL, "--config", databaseSeed}, "III_NAMESPACE="+namespace, "III_CONFIG_NAME="+namespace+"-database", "III_DISABLE_TRACE_PAYLOADS=1")
			smokeWait(t, "environment test database worker", func() bool {
				_, err := smokeValue(engine, engineURL, namespace, "database::listDatabases", map[string]any{})
				return err == nil
			})
			seed := DefaultConfig()
			seed.DataDir = filepath.Join(t.TempDir(), "accounts")
			seed.MaxSessions = 3
			seed.Cloud.Enabled = true
			seedPath := filepath.Join(dir, namespace+".yaml")
			encoded, err := json.Marshal(seed)
			if err != nil {
				t.Fatal(err)
			}
			if err = os.WriteFile(seedPath, encoded, 0600); err != nil {
				t.Fatal(err)
			}
			stored := seed
			if mode == "existing" {
				stored.MaxSessions = 6
				stored.Cloud.Enabled = false
				stored.Cloud.AccessToken = "saved-access-token"
				if _, err = smokeValue(engine, engineURL, "default", "configuration::ensure", map[string]any{
					"id": namespace, "name": "WhatsApp environment test", "description": "Isolated environment configuration test",
					"schema": configSchema, "initial_value": stored,
				}); err != nil {
					t.Fatal(err)
				}
			}
			args := []string{"--url", engineURL, "--config", seedPath}
			source := "central"
			if mode == "local" {
				args = append(args, "--local-config")
				source = "local"
			}
			childEnvironment := []string{
				"III_NAMESPACE=" + namespace, "III_CONFIG_NAME=" + namespace, "III_DISABLE_TRACE_PAYLOADS=1",
				"HTTPS_PROXY=http://127.0.0.1:9", "https_proxy=http://127.0.0.1:9",
				"NO_PROXY=127.0.0.1,localhost", "no_proxy=127.0.0.1,localhost",
			}
			for name, value := range environment {
				childEnvironment = append(childEnvironment, name+"="+value)
			}
			smokeProcess(t, dir, namespace, worker, args, childEnvironment...)
			smokeWait(t, "Cloud environment configuration", func() bool {
				status, err := smokeTrigger(engine, engineURL, namespace, "whatsapp::config-status", map[string]any{})
				return err == nil && status["source"] == source && status["last_outcome"] == "applied"
			})

			checkRuntime := func(wantSessions int) {
				t.Helper()
				status, err := smokeTrigger(engine, engineURL, namespace, "whatsapp::config-status", map[string]any{})
				if err != nil {
					t.Fatal(err)
				}
				if !reflect.DeepEqual(status["environment_overrides"], wantOverrides) {
					t.Fatal("runtime status must identify every environment-controlled Cloud field")
				}
				active, _ := status["active"].(map[string]any)
				cloud, _ := active["cloud"].(map[string]any)
				if active["max_sessions"] != float64(wantSessions) || cloud["enabled"] != true || cloud["api_version"] != "v25.0" {
					t.Fatal("runtime must combine saved operating limits with Cloud environment overrides")
				}
				for _, field := range []string{"access_token", "app_secret", "verify_token"} {
					if cloud[field] != "" {
						t.Fatal("active configuration must redact credentials")
					}
				}
				cloudStatus, err := smokeTrigger(engine, engineURL, namespace, "whatsapp::cloud::status", map[string]any{})
				if err != nil {
					t.Fatal(err)
				}
				if cloudStatus["configured"] != true || cloudStatus["webhook_ready"] != true ||
					cloudStatus["business_account_id"] != environment["WHATSAPP_CLOUD_BUSINESS_ACCOUNT_ID"] ||
					cloudStatus["phone_number_id"] != environment["WHATSAPP_CLOUD_PHONE_NUMBER_ID"] {
					t.Fatal("Cloud must use exact environment IDs and be ready without central credentials")
				}
				serialized, _ := json.Marshal([]any{status, cloudStatus})
				for _, name := range []string{"WHATSAPP_CLOUD_ACCESS_TOKEN", "WHATSAPP_META_APP_SECRET", "WHATSAPP_WEBHOOK_VERIFY_TOKEN"} {
					secret, _ := json.Marshal(environment[name])
					if strings.Contains(string(serialized), string(secret)) {
						t.Fatal("runtime status exposed an environment credential")
					}
				}
			}
			checkStored := func(want Config) {
				t.Helper()
				raw, err := smokeTrigger(engine, engineURL, "default", "configuration::get", map[string]any{"id": namespace, "raw": true})
				if err != nil {
					t.Fatal(err)
				}
				encoded, _ := json.Marshal(raw["value"])
				var got Config
				if json.Unmarshal(encoded, &got) != nil || !reflect.DeepEqual(got, want) {
					t.Fatal("worker environment or optional seed changed the stored configuration")
				}
			}
			checkRuntime(stored.MaxSessions)
			if mode == "local" {
				raw, err := smokeTrigger(engine, engineURL, "default", "configuration::get", map[string]any{"id": namespace, "raw": true})
				if err == nil && raw["value"] != nil {
					t.Fatal("local configuration must not publish a central seed")
				}
				return
			}
			checkStored(stored)
			stored.MaxSessions = 11
			stored.Cloud.Enabled = false
			stored.Cloud.APIVersion = "v26.0"
			stored.Cloud.AccessToken = "updated-saved-access-token"
			if _, err = smokeValue(engine, engineURL, "default", "configuration::set", map[string]any{"id": namespace, "value": stored}); err != nil {
				t.Fatal(err)
			}
			smokeWait(t, "Cloud environment survives central reload", func() bool {
				status, err := smokeTrigger(engine, engineURL, namespace, "whatsapp::config-status", map[string]any{})
				active, _ := status["active"].(map[string]any)
				return err == nil && active["max_sessions"] == float64(stored.MaxSessions) && status["last_outcome"] == "applied"
			})
			checkRuntime(stored.MaxSessions)
			checkStored(stored)
		})
	}
}
