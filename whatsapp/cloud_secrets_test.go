package main

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"sync"
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

const (
	testSecretToken  = "EAAG-vault-token"
	testSecretRotate = "EAAG-vault-rotated"
)

// A secrets worker double: refs maps a reference to its value or error.
func fakeResolver(refs map[string]any, calls *[]string) secretResolver {
	return func(_ context.Context, ref string) (string, error) {
		if calls != nil {
			*calls = append(*calls, ref)
		}
		switch v := refs[ref].(type) {
		case string:
			return v, nil
		case error:
			return "", v
		default:
			return "", &iii.InvocationError{Code: "SECRET_NOT_FOUND", Message: "no secret by that name"}
		}
	}
}

var secretsDown = &iii.InvocationError{Code: "function_not_found", Message: "function secrets::resolve not found"}

func referencedCloud() CloudConfig {
	c := DefaultCloudConfig()
	c.Enabled, c.BusinessAccountID, c.PhoneNumberID = true, "1234", "5678"
	c.AccessToken, c.AppSecret, c.VerifyToken = "secret://WA_TOKEN", "env://WA_APP_SECRET", "literal-verify"
	return c
}

func TestSecretRefParsing(t *testing.T) {
	for _, tc := range []struct {
		value, ref string
		isRef, bad bool
	}{
		{"secret://WA_TOKEN", "secret://WA_TOKEN", true, false},
		{"  SECRET://wa.token-2 ", "secret://wa.token-2", true, false},
		{"env://WHATSAPP_CLOUD_ACCESS_TOKEN", "env://WHATSAPP_CLOUD_ACCESS_TOKEN", true, false},
		{"secret://", "", true, true},
		{"secret://9starts-with-digit", "", true, true},
		{"env://has space", "", true, true},
		{"EAAG-literal", "", false, false},
		{"${WHATSAPP_TOKEN}", "", false, false},
		{"", "", false, false},
	} {
		ref, isRef, err := secretRef(tc.value)
		if isRef != tc.isRef || (err != nil) != tc.bad || ref != tc.ref {
			t.Errorf("secretRef(%q) = %q, %t, %v", tc.value, ref, isRef, err)
		}
	}
}

func TestStoredReferencesValidate(t *testing.T) {
	c := referencedCloud()
	c.FlowPrivateKey = "secret://WA_FLOW_KEY"
	if err := c.Validate(); err != nil {
		t.Fatalf("references rejected: %v", err)
	}
	c.AccessToken = "secret://not a name"
	err := c.Validate()
	if err == nil || !strings.Contains(err.Error(), "cloud.access_token") || strings.Contains(err.Error(), "not a name") {
		t.Fatalf("invalid reference: %v", err)
	}
}

func TestResolveReplacesReferencesOnly(t *testing.T) {
	var calls []string
	resolve := fakeResolver(map[string]any{"secret://WA_TOKEN": testSecretToken, "env://WA_APP_SECRET": "app-secret"}, &calls)
	got := resolveCloudCredentials(context.Background(), resolve, referencedCloud(), CloudConfig{}, "")
	if got.AccessToken != testSecretToken || got.AppSecret != "app-secret" || got.VerifyToken != "literal-verify" {
		t.Fatalf("resolved %q %q %q", got.AccessToken, got.AppSecret, got.VerifyToken)
	}
	if len(calls) != 2 {
		t.Fatalf("resolve calls: %v", calls)
	}
	if problems := got.credentialProblems(); len(problems) != 0 {
		t.Fatalf("problems: %v", problems)
	}
	if err := got.Validate(); err != nil {
		t.Fatal(err)
	}
	// A configuration without references never calls the secrets worker.
	calls = nil
	literal := DefaultCloudConfig()
	literal.AccessToken = "EAAG-literal"
	if resolveCloudCredentials(context.Background(), resolve, literal, CloudConfig{}, "") != literal || len(calls) != 0 {
		t.Fatalf("literal configuration changed or resolved: %v", calls)
	}
}

func TestRefusedReferenceIsDroppedWithItsReason(t *testing.T) {
	forbidden := &iii.InvocationError{Code: "SECRET_FORBIDDEN", Message: "not a consumer"}
	resolve := fakeResolver(map[string]any{"secret://WA_TOKEN": forbidden, "env://WA_APP_SECRET": "app-secret"}, nil)
	previous := resolveCloudCredentials(context.Background(), fakeResolver(map[string]any{"secret://WA_TOKEN": testSecretToken, "env://WA_APP_SECRET": "app-secret"}, nil), referencedCloud(), CloudConfig{}, "")
	// Revocation must take effect: the last value is not kept.
	got := resolveCloudCredentials(context.Background(), resolve, referencedCloud(), previous, "")
	if got.AccessToken != "" {
		t.Fatal("revoked token kept")
	}
	problem := got.credentialProblems()["access_token"]
	if !strings.Contains(problem, "secret://WA_TOKEN") || !strings.Contains(problem, "consumers") {
		t.Fatalf("problem: %q", problem)
	}
	// The configuration stays valid, so the worker starts and reports it.
	if err := got.Validate(); err != nil {
		t.Fatalf("unresolved credential rejected the configuration: %v", err)
	}
	status := cloudStatus(Config{Cloud: got})
	if status.Configured || status.CredentialErrors["access_token"] != problem {
		t.Fatalf("status: %+v", status)
	}
	err := cloudRequestConfig(Config{Cloud: got})
	var invocation *iii.InvocationError
	if !errors.As(err, &invocation) || invocation.Code != "cloud_not_configured" || !strings.Contains(invocation.Message, "secret://WA_TOKEN") {
		t.Fatalf("request error: %v", err)
	}
}

func TestUnreachableSecretsWorkerKeepsTheLastValue(t *testing.T) {
	resolve := fakeResolver(map[string]any{"secret://WA_TOKEN": testSecretToken, "env://WA_APP_SECRET": "app-secret"}, nil)
	previous := resolveCloudCredentials(context.Background(), resolve, referencedCloud(), CloudConfig{}, "")
	down := fakeResolver(map[string]any{"secret://WA_TOKEN": secretsDown, "env://WA_APP_SECRET": secretsDown}, nil)

	// Nothing can rotate or revoke a secret while its worker is down.
	kept := resolveCloudCredentials(context.Background(), down, referencedCloud(), previous, "")
	if kept.AccessToken != testSecretToken || kept.AppSecret != "app-secret" {
		t.Fatal("last values dropped while the secrets worker is down")
	}
	if !strings.Contains(kept.credentialProblems()["access_token"], "not running") {
		t.Fatalf("problem: %v", kept.credentialProblems())
	}
	// A reference secrets::on-changed named is suspect: its old value is not kept.
	suspect := resolveCloudCredentials(context.Background(), down, referencedCloud(), previous, "secret://WA_TOKEN")
	if suspect.AccessToken != "" || suspect.AppSecret != "app-secret" {
		t.Fatalf("suspect handling: token %q, app secret %q", suspect.AccessToken, suspect.AppSecret)
	}
	// A changed reference has no last value of its own.
	moved := referencedCloud()
	moved.AccessToken = "secret://WA_OTHER"
	if got := resolveCloudCredentials(context.Background(), down, moved, previous, ""); got.AccessToken != "" {
		t.Fatal("another reference's value reused")
	}
	// At startup there is nothing to keep.
	if got := resolveCloudCredentials(context.Background(), down, referencedCloud(), CloudConfig{}, ""); got.AccessToken != "" || got.Validate() != nil {
		t.Fatalf("startup without the secrets worker: %v", got.Validate())
	}
}

func TestResolvedValuesAreCheckedLikeLiterals(t *testing.T) {
	c := referencedCloud()
	c.FlowPrivateKey = "secret://WA_FLOW_KEY"
	resolve := fakeResolver(map[string]any{"secret://WA_TOKEN": "two\nlines", "env://WA_APP_SECRET": "", "secret://WA_FLOW_KEY": "not a key"}, nil)
	got := resolveCloudCredentials(context.Background(), resolve, c, CloudConfig{}, "")
	if got.AccessToken != "" || got.AppSecret != "" || got.FlowPrivateKey != "" {
		t.Fatal("invalid resolved values used")
	}
	problems := got.credentialProblems()
	for _, field := range []string{"access_token", "app_secret", "flow_private_key"} {
		if problems[field] == "" {
			t.Errorf("%s: no problem reported", field)
		}
		if strings.Contains(problems[field], "two") || strings.Contains(problems[field], "not a key") {
			t.Errorf("%s: value leaked into %q", field, problems[field])
		}
	}
	if err := got.Validate(); err != nil {
		t.Fatal(err)
	}
}

func TestRotationChangesTheCloudConfiguration(t *testing.T) {
	first := resolveCloudCredentials(context.Background(), fakeResolver(map[string]any{"secret://WA_TOKEN": testSecretToken, "env://WA_APP_SECRET": "app-secret"}, nil), referencedCloud(), CloudConfig{}, "")
	rotated := resolveCloudCredentials(context.Background(), fakeResolver(map[string]any{"secret://WA_TOKEN": testSecretRotate, "env://WA_APP_SECRET": "app-secret"}, nil), referencedCloud(), first, "secret://WA_TOKEN")
	if rotated.AccessToken != testSecretRotate || rotated == first {
		t.Fatal("rotation not applied")
	}
	if !first.referencesSecret("secret://WA_TOKEN") || first.referencesSecret("secret://OTHER") {
		t.Fatal("referenced secrets")
	}
}

func TestConfigurationReloadResolvesAndFollowsSecretChanges(t *testing.T) {
	clearCloudEnvironment(t)
	t.Setenv("III_NAMESPACE", "team-a")
	c := DefaultConfig()
	c.DataDir = t.TempDir()
	c.Cloud = referencedCloud()
	c.Cloud.AppSecret = "literal-app-secret"
	var mu sync.Mutex
	current, resolves, bound, resolvedIn := testSecretToken, 0, false, ""
	peer := newCallbackPeer(t, func(msg map[string]json.RawMessage) map[string]any {
		mu.Lock()
		defer mu.Unlock()
		switch {
		case string(msg["type"]) == `"observe"`:
			var frame struct {
				Source struct {
					FunctionID string `json:"function_id"`
					Namespace  string `json:"namespace"`
					Deliver    string `json:"deliver"`
				} `json:"source"`
				Target struct {
					FunctionID string `json:"function_id"`
				} `json:"target"`
			}
			if json.Unmarshal(msg["source"], &frame.Source) == nil && json.Unmarshal(msg["target"], &frame.Target) == nil && frame.Source.FunctionID == "secrets::on-changed" {
				bound = frame.Target.FunctionID == "whatsapp::on-secret-changed" && frame.Source.Namespace == "team-a" && frame.Source.Deliver == "output"
			}
		case string(msg["function_id"]) == `"configuration::get"` && len(msg["invocation_id"]) > 0:
			reply := callbackInvocationReply(msg)
			reply["result"] = map[string]any{"value": c}
			return reply
		case string(msg["function_id"]) == `"secrets::resolve"` && len(msg["invocation_id"]) > 0:
			resolves++
			resolvedIn = string(msg["namespace"])
			reply := callbackInvocationReply(msg)
			reply["result"] = map[string]any{"name": "WA_TOKEN", "value": current}
			return reply
		}
		return nil
	})
	// Observers go out once the engine has registered the worker.
	peer.registerWorker(t)
	manager, err := newTestManager(c)
	if err != nil {
		t.Fatal(err)
	}
	defer manager.Shutdown(context.Background())
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if err := registerConfiguration(ctx, peer.client, manager, false); err != nil {
		t.Fatal(err)
	}
	if got := manager.Config().Cloud.AccessToken; got != testSecretToken {
		t.Fatalf("reload left access token %q", got)
	}
	status, remote, err := peer.invoke(ctx, "whatsapp::config-status", json.RawMessage(`{}`))
	if err != nil || remote != nil || strings.Contains(string(status), testSecretToken) {
		t.Fatalf("config status: %s %v %v", status, remote, err)
	}

	mu.Lock()
	current = testSecretRotate
	before := resolves
	mu.Unlock()
	if _, remote, err := peer.invoke(ctx, "whatsapp::on-secret-changed", json.RawMessage(`{"name":"OTHER","ref":"secret://OTHER","action":"rotated"}`)); err != nil || remote != nil {
		t.Fatal(remote, err)
	}
	mu.Lock()
	unrelated := resolves - before
	mu.Unlock()
	if unrelated != 0 || manager.Config().Cloud.AccessToken != testSecretToken {
		t.Fatalf("an unrelated secret caused %d resolves", unrelated)
	}
	if _, remote, err := peer.invoke(ctx, "whatsapp::on-secret-changed", json.RawMessage(`{"name":"WA_TOKEN","ref":"secret://WA_TOKEN","action":"rotated"}`)); err != nil || remote != nil {
		t.Fatal(remote, err)
	}
	if got := manager.Config().Cloud.AccessToken; got != testSecretRotate {
		t.Fatalf("rotation not followed: %q", got)
	}
	mu.Lock()
	defer mu.Unlock()
	if !bound {
		t.Fatal("whatsapp::on-secret-changed does not observe secrets::on-changed in team-a")
	}
	// The secrets worker admits only callers from its own namespace.
	if resolvedIn != `"team-a"` {
		t.Fatalf("secrets::resolve called in namespace %s", resolvedIn)
	}
}
