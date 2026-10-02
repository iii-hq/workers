package main

import (
	"encoding/json"
	"os"
	"reflect"
	"strconv"
	"strings"
	"testing"
)

func clearCloudEnvironment(t *testing.T) {
	t.Helper()
	for _, name := range []string{
		"WHATSAPP_CLOUD_ENABLED", "WHATSAPP_CLOUD_API_VERSION",
		"WHATSAPP_CLOUD_BUSINESS_ACCOUNT_ID", "WHATSAPP_CLOUD_PHONE_NUMBER_ID",
		"WHATSAPP_CLOUD_ACCESS_TOKEN", "WHATSAPP_META_APP_SECRET", "WHATSAPP_WEBHOOK_VERIFY_TOKEN",
	} {
		t.Setenv(name, "")
		if err := os.Unsetenv(name); err != nil {
			t.Fatal(err)
		}
	}
}

func TestCloudEnvironmentOverridesStoredValues(t *testing.T) {
	clearCloudEnvironment(t)
	t.Setenv("WHATSAPP_CLOUD_ENABLED", "true")
	t.Setenv("WHATSAPP_CLOUD_API_VERSION", "v25.0")
	t.Setenv("WHATSAPP_CLOUD_BUSINESS_ACCOUNT_ID", "00012345678901234567890123456789")
	t.Setenv("WHATSAPP_CLOUD_PHONE_NUMBER_ID", "98765432109876543210987654321098")
	t.Setenv("WHATSAPP_CLOUD_ACCESS_TOKEN", `env-secret: "quoted" ${DO_NOT_EXPAND}`)
	t.Setenv("WHATSAPP_META_APP_SECRET", "true")
	t.Setenv("WHATSAPP_WEBHOOK_VERIFY_TOKEN", "00123")
	t.Setenv("DO_NOT_EXPAND", "must-not-appear")
	c, err := parseConfigJSON([]byte(`{"max_sessions":9,"cloud":{"enabled":false,"api_version":"v26.0","access_token":"stored-token"}}`))
	if err != nil {
		t.Fatal(err)
	}
	want := CloudConfig{
		Enabled: true, APIVersion: "v25.0",
		BusinessAccountID: "00012345678901234567890123456789", PhoneNumberID: "98765432109876543210987654321098",
		AccessToken: `env-secret: "quoted" ${DO_NOT_EXPAND}`, AppSecret: "true", VerifyToken: "00123",
	}
	if !reflect.DeepEqual(c.Cloud, want) || c.MaxSessions != 9 {
		t.Fatal("worker environment must override Cloud fields without coercion or expansion")
	}
	redacted, _ := json.Marshal(c.Redacted())
	if strings.Contains(string(redacted), "env-secret") || strings.Contains(string(redacted), "DO_NOT_EXPAND") {
		t.Fatal("environment credential leaked into redacted configuration")
	}
}

func TestCloudEnvironmentUnsetAndEmptyValues(t *testing.T) {
	clearCloudEnvironment(t)
	input := []byte(`{"cloud":{"access_token":"stored-token","app_secret":"stored-secret","verify_token":"stored-verify"}}`)
	c, err := parseConfigJSON(input)
	if err != nil || c.Cloud.AccessToken != "stored-token" || c.Cloud.APIVersion != "v26.0" {
		t.Fatal("unset variables must preserve saved values and defaults")
	}
	t.Setenv("WHATSAPP_CLOUD_ACCESS_TOKEN", "")
	t.Setenv("WHATSAPP_META_APP_SECRET", "")
	t.Setenv("WHATSAPP_WEBHOOK_VERIFY_TOKEN", "")
	c, err = parseConfigJSON(input)
	if err != nil || c.Cloud.AccessToken != "" || c.Cloud.AppSecret != "" || c.Cloud.VerifyToken != "" {
		t.Fatal("explicit empty environment values must clear saved credentials")
	}
	t.Setenv("WHATSAPP_CLOUD_ENABLED", "false")
	c, err = parseConfigJSON([]byte(`{"cloud":{"enabled":true}}`))
	if err != nil || c.Cloud.Enabled {
		t.Fatal("an explicit false override must disable Cloud API before validation")
	}
}

func TestCloudEnvironmentValidationDoesNotExposeValues(t *testing.T) {
	for _, tc := range []struct{ name, value string }{
		{"WHATSAPP_CLOUD_ENABLED", "private-sentinel"},
		{"WHATSAPP_CLOUD_ENABLED", ""},
		{"WHATSAPP_CLOUD_ENABLED", "1"},
		{"WHATSAPP_CLOUD_API_VERSION", "private-sentinel"},
		{"WHATSAPP_CLOUD_BUSINESS_ACCOUNT_ID", "private-sentinel"},
		{"WHATSAPP_CLOUD_PHONE_NUMBER_ID", "private-sentinel"},
		{"WHATSAPP_CLOUD_ACCESS_TOKEN", "private-sentinel\n"},
		{"WHATSAPP_META_APP_SECRET", "private-sentinel\r"},
		{"WHATSAPP_WEBHOOK_VERIFY_TOKEN", strings.Repeat("private-sentinel", 2000)},
	} {
		t.Run(tc.name+"/"+strconv.Itoa(len(tc.value)), func(t *testing.T) {
			clearCloudEnvironment(t)
			t.Setenv(tc.name, tc.value)
			_, err := parseConfigJSON([]byte(`{}`))
			if err == nil || strings.Contains(err.Error(), "private-sentinel") {
				t.Fatal("invalid environment must be rejected without exposing its value")
			}
		})
	}
	t.Run("enabled requires credentials", func(t *testing.T) {
		clearCloudEnvironment(t)
		t.Setenv("WHATSAPP_CLOUD_ENABLED", "true")
		if _, err := parseConfigJSON([]byte(`{}`)); err == nil {
			t.Fatal("enabling Cloud API must validate the final effective configuration")
		}
	})
}

func TestCloudEnvironmentNeverEntersConfigurationSeed(t *testing.T) {
	clearCloudEnvironment(t)
	t.Setenv("WHATSAPP_CLOUD_BUSINESS_ACCOUNT_ID", "1234567")
	t.Setenv("WHATSAPP_CLOUD_PHONE_NUMBER_ID", "7654321")
	t.Setenv("WHATSAPP_CLOUD_ACCESS_TOKEN", "private-sentinel")
	seed, err := parseConfigYAML([]byte("data_dir: custom-data\ncloud:\n  enabled: true\n"))
	if err != nil {
		t.Fatal("a seed may rely on runtime environment credentials:", err)
	}
	if !seed.Cloud.Enabled || seed.DataDir != "custom-data" || seed.Cloud.BusinessAccountID != "" || seed.Cloud.PhoneNumberID != "" || seed.Cloud.AccessToken != "" {
		t.Fatal("runtime environment values must not enter the seed sent to configuration::ensure")
	}
	encoded, _ := json.Marshal(seed)
	effective, err := parseConfigJSON(encoded)
	if err != nil || effective.Cloud.AccessToken != "private-sentinel" {
		t.Fatal("runtime configuration must apply environment after reading the seed")
	}
}

func TestCentralCloudValuesAreNotExpandedAgain(t *testing.T) {
	clearCloudEnvironment(t)
	t.Setenv("LITERAL_SECRET", "must-not-appear")
	c, err := parseConfigJSON([]byte(`{"cloud":{"access_token":"${LITERAL_SECRET}"}}`))
	if err != nil || c.Cloud.AccessToken != "${LITERAL_SECRET}" {
		t.Fatal("already resolved central values must remain literal")
	}
}
