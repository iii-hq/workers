package main

import (
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// A Cloud access token kept in the secrets worker: WhatsApp resolves its
// secret:// reference as consumer "whatsapp", reports why when the secret
// stops being shared with it, and picks the value up again once it is.
func TestCloudSecretReferencesEngine(t *testing.T) {
	engine, worker := os.Getenv("III_BINARY"), os.Getenv("WHATSAPP_BINARY")
	database, secrets := os.Getenv("DATABASE_BINARY"), os.Getenv("SECRETS_BINARY")
	if engine == "" || worker == "" || database == "" || secrets == "" {
		t.Skip("set III_BINARY, WHATSAPP_BINARY, DATABASE_BINARY and SECRETS_BINARY for isolated loopback integration")
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
	smokeWait(t, "secrets test engine", func() bool {
		_, err := smokeValue(engine, engineURL, "default", "engine::workers::list", map[string]any{})
		return err == nil
	})

	key := make([]byte, 32)
	if _, err = rand.Read(key); err != nil {
		t.Fatal(err)
	}
	// The secrets worker serves callers from its own namespace only, so it
	// runs in WhatsApp's.
	namespace := "whatsapp-secrets"
	smokeProcess(t, dir, "secrets", secrets, []string{"--url", engineURL}, "III_NAMESPACE="+namespace, "III_SECRETS_KEY="+base64.StdEncoding.EncodeToString(key))
	smokeWait(t, "secrets worker", func() bool {
		_, err := smokeValue(engine, engineURL, namespace, "secrets::status", map[string]any{})
		return err == nil
	})
	const token = "EAAG-engine-secret-token"
	if _, err = smokeValue(engine, engineURL, namespace, "secrets::set", map[string]any{"name": "WA_TOKEN", "value": token, "consumers": []string{"whatsapp"}}); err != nil {
		t.Fatal(err)
	}

	databaseSeed := filepath.Join(dir, "database.yaml")
	if err = os.WriteFile(databaseSeed, []byte("databases:\n  whatsapp:\n    url: sqlite:"+filepath.Join(t.TempDir(), "whatsapp.db")+"\n    pool:\n      max: 1\nhistory_max_entries: 0\nhistory_max_bytes: 0\n"), 0600); err != nil {
		t.Fatal(err)
	}
	smokeProcess(t, dir, "database", database, []string{"--url", engineURL, "--config", databaseSeed}, "III_NAMESPACE="+namespace, "III_CONFIG_NAME="+namespace+"-database", "III_DISABLE_TRACE_PAYLOADS=1")
	smokeWait(t, "database worker", func() bool {
		_, err := smokeValue(engine, engineURL, namespace, "database::listDatabases", map[string]any{})
		return err == nil
	})
	seed := DefaultConfig()
	seed.DataDir = filepath.Join(t.TempDir(), "accounts")
	seed.Cloud.Enabled, seed.Cloud.BusinessAccountID, seed.Cloud.PhoneNumberID = true, "123456789012345", "123456789012346"
	seed.Cloud.AccessToken, seed.Cloud.AppSecret, seed.Cloud.VerifyToken = "secret://WA_TOKEN", "app-secret", "verify-token"
	seedPath := filepath.Join(dir, "whatsapp.json")
	encoded, err := json.Marshal(seed)
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(seedPath, encoded, 0600); err != nil {
		t.Fatal(err)
	}
	smokeProcess(t, dir, "whatsapp", worker, []string{"--url", engineURL, "--config", seedPath},
		"III_NAMESPACE="+namespace, "III_CONFIG_NAME="+namespace, "III_DISABLE_TRACE_PAYLOADS=1",
		"HTTPS_PROXY=http://127.0.0.1:9", "https_proxy=http://127.0.0.1:9", "NO_PROXY=127.0.0.1,localhost", "no_proxy=127.0.0.1,localhost")

	var last any
	t.Cleanup(func() {
		if t.Failed() {
			t.Logf("last Cloud status: %v", last)
		}
	})
	cloudStatus := func() map[string]any {
		status, err := smokeTrigger(engine, engineURL, namespace, "whatsapp::cloud::status", map[string]any{})
		if err != nil {
			last = err
			return nil
		}
		last = status
		if strings.Contains(fmt.Sprint(status), token) {
			t.Fatal("Cloud status exposed the resolved token")
		}
		return status
	}
	problem := func(status map[string]any) string {
		errors, _ := status["credential_errors"].(map[string]any)
		text, _ := errors["access_token"].(string)
		return text
	}
	smokeWait(t, "resolved access token", func() bool {
		status := cloudStatus()
		return status != nil && status["access_token_set"] == true && status["configured"] == true && problem(status) == ""
	})
	stored, err := smokeTrigger(engine, engineURL, "default", "configuration::get", map[string]any{"id": namespace, "raw": true})
	if err != nil || strings.Contains(fmt.Sprint(stored), token) {
		t.Fatalf("stored configuration must hold only the reference: %v", err)
	}

	// Revoking WhatsApp's access reaches it through secrets::changed.
	if _, err = smokeValue(engine, engineURL, namespace, "secrets::access", map[string]any{"name": "WA_TOKEN", "consumers": []string{}}); err != nil {
		t.Fatal(err)
	}
	smokeWait(t, "revoked access token", func() bool {
		status := cloudStatus()
		return status != nil && status["access_token_set"] == false && strings.Contains(problem(status), "add whatsapp to its consumers")
	})
	if _, err = smokeValue(engine, engineURL, namespace, "secrets::access", map[string]any{"name": "WA_TOKEN", "consumers": []string{"whatsapp"}}); err != nil {
		t.Fatal(err)
	}
	smokeWait(t, "shared again", func() bool {
		status := cloudStatus()
		return status != nil && status["access_token_set"] == true && problem(status) == ""
	})
}
