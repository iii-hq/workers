package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"regexp"
	"strings"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

// Cloud credentials may be references instead of values: secret://NAME (the
// secrets worker's encrypted vault) or env://NAME (a variable the secrets
// worker reads). They are resolved through secrets::resolve, which admits the
// worker only when the secret lists "whatsapp" among its consumers. The stored
// configuration keeps the reference; the value lives only in the active
// configuration and never reaches a status, log or reply.

const (
	secretsResolveTimeout = 3 * time.Second
	// While a reference is unresolved it is retried this often; resolved ones
	// are re-read at the slower interval in case a secrets::changed was missed.
	secretsRetryInterval   = 10 * time.Second
	secretsRefreshInterval = 5 * time.Minute
)

var secretRefNamePattern = regexp.MustCompile(`^[A-Za-z_][A-Za-z0-9_.-]{0,127}$`)

// secretRef reports whether value is a secret:// or env:// reference and
// returns it with the scheme in lower case. A reference with an invalid name is
// an error that does not repeat the value.
func secretRef(value string) (string, bool, error) {
	value = strings.TrimSpace(value)
	for _, scheme := range []string{"secret://", "env://"} {
		if len(value) < len(scheme) || !strings.EqualFold(value[:len(scheme)], scheme) {
			continue
		}
		name := value[len(scheme):]
		if !secretRefNamePattern.MatchString(name) {
			return "", true, errors.New("must be a secret://NAME or env://NAME reference whose NAME starts with a letter or underscore and has at most 128 letters, digits, '_', '.' or '-'")
		}
		return scheme + name, true, nil
	}
	return "", false, nil
}

// cloudCredential is where one credential came from: ref is its reference, and
// problem says why it is unresolved (the value is then empty) or why its last
// value is kept.
type cloudCredential struct{ ref, problem string }

var cloudCredentialNames = [4]string{"access_token", "app_secret", "verify_token", "flow_private_key"}

func (c *CloudConfig) credentialValues() [4]*string {
	return [4]*string{&c.AccessToken, &c.AppSecret, &c.VerifyToken, &c.FlowPrivateKey}
}

// credentialProblems maps a credential field to why its reference is not
// resolved, or why its last value is still in use.
func (c CloudConfig) credentialProblems() map[string]string {
	problems := map[string]string{}
	for i, credential := range c.credentials {
		if credential.problem != "" {
			problems[cloudCredentialNames[i]] = credential.problem
		}
	}
	return problems
}

func (c CloudConfig) referencesSecret(ref string) bool {
	for _, credential := range c.credentials {
		if credential.ref != "" && credential.ref == ref {
			return true
		}
	}
	return false
}

func (c CloudConfig) hasSecretRefs() bool {
	for _, credential := range c.credentials {
		if credential.ref != "" {
			return true
		}
	}
	return false
}

type secretResolver func(ctx context.Context, ref string) (string, error)

// busSecretResolver calls secrets::resolve in this worker's namespace: the
// secrets worker admits only callers from its own, and the engine stamps this
// worker's name on the call, which is what it checks against consumers.
func busSecretResolver(client *iii.Client) secretResolver {
	return func(ctx context.Context, ref string) (string, error) {
		data, err := json.Marshal(map[string]string{"ref": ref})
		if err != nil {
			return "", err
		}
		result, err := client.Trigger(ctx, iii.TriggerRequest{FunctionID: "secrets::resolve", Data: data, Timeout: secretsResolveTimeout, Namespace: Namespace()})
		if err != nil {
			return "", err
		}
		var resolved struct {
			Value string `json:"value"`
		}
		if json.Unmarshal(result, &resolved) != nil {
			return "", errors.New("malformed secrets::resolve response")
		}
		return resolved.Value, nil
	}
}

// resolveCloudCredentials returns next with each referenced credential
// replaced by its value. previous is the active configuration: while the
// secrets worker cannot be reached a reference keeps its last value, unless
// it is suspect (a secrets::changed event named it). A refusal from the secrets
// worker always drops the value, so revoking access takes effect.
func resolveCloudCredentials(ctx context.Context, resolve secretResolver, next, previous CloudConfig, suspect string) CloudConfig {
	values, last := next.credentialValues(), previous.credentialValues()
	for i, name := range cloudCredentialNames {
		ref, isRef, err := secretRef(*values[i])
		if !isRef || err != nil {
			next.credentials[i] = cloudCredential{}
			continue
		}
		next.credentials[i] = cloudCredential{ref: ref}
		*values[i] = ""
		value, err := resolve(ctx, ref)
		if err == nil {
			if reason := checkResolvedCredential(name, value); reason != "" {
				next.credentials[i].problem = "the value of " + ref + " " + reason
			} else {
				*values[i] = value
			}
			continue
		}
		problem, transient := secretProblem(ref, err)
		kept := previous.credentials[i]
		if transient && ref != suspect && kept.ref == ref && *last[i] != "" {
			*values[i] = *last[i]
			problem += "; using the last value read"
		}
		next.credentials[i].problem = problem
	}
	return next
}

// checkResolvedCredential applies the checks a literal gets; the reason never
// quotes the value.
func checkResolvedCredential(name, value string) string {
	if strings.TrimSpace(value) == "" {
		return "is empty"
	}
	if name == "flow_private_key" {
		if _, err := cloudFlowPrivateKey(value); err != nil {
			return "is not an RSA private key in PEM format"
		}
		return ""
	}
	if len(value) > 16384 || strings.ContainsAny(value, "\r\n") {
		return "is not a single line of at most 16384 bytes"
	}
	return ""
}

// secretProblem describes a failed resolution without the value, and says
// whether it may be temporary (the secrets worker is not reachable).
func secretProblem(ref string, err error) (string, bool) {
	var invocation *iii.InvocationError
	switch {
	case errors.As(err, &invocation) && invocation.Code == "function_not_found":
		return ref + ": the secrets worker is not running", true
	case errors.As(err, &invocation) && invocation.Code == "SECRET_FORBIDDEN":
		return ref + " is not shared with the whatsapp worker; add whatsapp to its consumers", false
	case errors.As(err, &invocation) && invocation.Code == "SECRET_NOT_FOUND":
		return ref + " is not in the secrets store", false
	case errors.As(err, &invocation):
		return fmt.Sprintf("%s could not be resolved (%s)", ref, invocation.Code), false
	case errors.Is(err, iii.ErrTimeout), errors.Is(err, context.DeadlineExceeded):
		return ref + ": the secrets worker did not answer", true
	default:
		return ref + ": the secrets worker could not be reached", true
	}
}
