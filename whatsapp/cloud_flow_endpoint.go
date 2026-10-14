package main

import (
	"context"
	"crypto/aes"
	"crypto/cipher"
	"crypto/rand"
	"crypto/rsa"
	"crypto/sha256"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"errors"
	"strings"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

const cloudFlowEnvelopeLimit = 256 * 1024

func cloudFlowPrivateKey(encoded string) (*rsa.PrivateKey, error) {
	invalid := errors.New("cloud.flow_private_key must be an unencrypted PEM RSA private key with 2048 bits")
	if len(encoded) > 16384 {
		return nil, invalid
	}
	block, rest := pem.Decode([]byte(encoded))
	if block == nil || len(strings.TrimSpace(string(rest))) != 0 {
		return nil, invalid
	}
	var key *rsa.PrivateKey
	if block.Type == "RSA PRIVATE KEY" {
		key, _ = x509.ParsePKCS1PrivateKey(block.Bytes)
	} else if block.Type == "PRIVATE KEY" {
		parsed, _ := x509.ParsePKCS8PrivateKey(block.Bytes)
		key, _ = parsed.(*rsa.PrivateKey)
	}
	if key == nil || key.N.BitLen() != 2048 || key.Validate() != nil {
		return nil, invalid
	}
	return key, nil
}

func cloudFlowPath() string {
	return strings.Replace(cloudWebhookPath(), "/webhooks/whatsapp-cloud/", "/webhooks/whatsapp-flows/", 1)
}

// This wrapper does not log keys, decrypted payloads or encryption errors.
// Plaintext application data goes only to the configured iii handler, which
// must apply its own logging and trace-payload policy.
func processCloudFlow(ctx context.Context, config Config, raw []byte, signature string, handler func(context.Context, json.RawMessage) (json.RawMessage, error)) cloudHTTPResponse {
	if !config.Cloud.Enabled || config.Cloud.AppSecret == "" || config.Cloud.FlowPrivateKey == "" {
		return cloudHTTPText(503, "Flow endpoint is not configured")
	}
	if len(raw) > cloudFlowEnvelopeLimit {
		return cloudHTTPText(413, "Flow payload exceeds the size limit")
	}
	if !verifyCloudSignature(config.Cloud.AppSecret, raw, signature) {
		return cloudHTTPText(403, "Invalid signature")
	}
	key, err := cloudFlowPrivateKey(config.Cloud.FlowPrivateKey)
	if err != nil {
		return cloudHTTPText(503, "Flow encryption is not configured")
	}
	var envelope struct {
		EncryptedFlowData string `json:"encrypted_flow_data"`
		EncryptedAESKey   string `json:"encrypted_aes_key"`
		InitialVector     string `json:"initial_vector"`
	}
	if json.Unmarshal(raw, &envelope) != nil {
		return cloudHTTPText(400, "Invalid Flow envelope")
	}
	encodedKey, err := base64.StdEncoding.Strict().DecodeString(envelope.EncryptedAESKey)
	if err != nil || len(encodedKey) != key.Size() {
		return cloudHTTPText(421, "Unable to decrypt Flow request")
	}
	aesKey, err := rsa.DecryptOAEP(sha256.New(), rand.Reader, key, encodedKey, nil)
	if err != nil || len(aesKey) != 16 {
		return cloudHTTPText(421, "Unable to decrypt Flow request")
	}
	iv, err := base64.StdEncoding.Strict().DecodeString(envelope.InitialVector)
	if err != nil || len(iv) != 16 {
		return cloudHTTPText(421, "Unable to decrypt Flow request")
	}
	encrypted, err := base64.StdEncoding.Strict().DecodeString(envelope.EncryptedFlowData)
	if err != nil || len(encrypted) > 65536+16 {
		return cloudHTTPText(421, "Unable to decrypt Flow request")
	}
	block, _ := aes.NewCipher(aesKey)
	gcm, _ := cipher.NewGCMWithNonceSize(block, len(iv))
	plain, err := gcm.Open(nil, iv, encrypted, nil)
	if err != nil {
		return cloudHTTPText(421, "Unable to decrypt Flow request")
	}
	var payload map[string]any
	if json.Unmarshal(plain, &payload) != nil || payload["version"] != "3.0" {
		return cloudHTTPText(400, "Unsupported Flow payload or data API version")
	}
	var response json.RawMessage
	if payload["action"] == "ping" {
		response = json.RawMessage(`{"data":{"status":"active"}}`)
	} else {
		action, _ := payload["action"].(string)
		if action != "INIT" && action != "BACK" && action != "data_exchange" {
			return cloudHTTPText(400, "Unsupported Flow action")
		}
		token, _ := payload["flow_token"].(string)
		if len(token) == 0 || len(token) > 4096 {
			return cloudHTTPText(400, "Invalid Flow token")
		}
		if notification, exists := cloudNested(payload, "data")["error"]; exists {
			code, ok := notification.(string)
			if !ok || len(code) == 0 || len(code) > 1024 || (action != "INIT" && action != "data_exchange") {
				return cloudHTTPText(400, "Invalid Flow error notification")
			}
			// Meta requires an encrypted acknowledgement without a screen.
			// Do not run business actions again in response to a client error.
			response = json.RawMessage(`{"data":{"acknowledged":true}}`)
		} else {
			if config.Cloud.FlowHandlerFunction == "" || handler == nil {
				return cloudHTTPText(503, "Flow business handler is not configured")
			}
			callCtx, cancel := context.WithTimeout(ctx, 8*time.Second)
			response, err = handler(callCtx, json.RawMessage(plain))
			cancel()
			if err != nil {
				return cloudHTTPText(503, "Flow business handler failed")
			}
			var value map[string]any
			if len(response) > 65536 || json.Unmarshal(response, &value) != nil || value == nil {
				return cloudHTTPText(502, "Flow handler returned an invalid response")
			}
			screen, ok := value["screen"].(string)
			if !ok || len(screen) == 0 || len(screen) > 128 {
				return cloudHTTPText(502, "Flow handler must return a screen")
			}
			if _, ok := value["data"].(map[string]any); !ok {
				return cloudHTTPText(502, "Flow handler must return a data object")
			}
			response, err = json.Marshal(cloudSanitizeResult(config, value))
			if err != nil {
				return cloudHTTPText(502, "Invalid Flow response")
			}
		}
	}
	for i := range iv {
		iv[i] ^= 0xff
	}
	return cloudHTTPText(200, base64.StdEncoding.EncodeToString(gcm.Seal(nil, iv, response, nil)))
}

func registerCloudFlowEndpoint(client *iii.Client, manager *Manager, engineURL string) error {
	const function = "whatsapp::cloud::flows::endpoint"
	metadata := iii.RegisterFunctionOptions{Metadata: json.RawMessage(`{"internal":true,"trace_hidden":true}`)}
	if err := iii.RegisterFunctionTyped(client, function, cloudFlowEndpointHandler(client, manager, engineURL), metadata); err != nil {
		return err
	}
	route, _ := json.Marshal(map[string]any{"api_path": cloudFlowPath(), "http_method": "POST", "public_webhook": true})
	return client.RegisterTriggerNamespaced(ConfigID()+"-cloud-flow-endpoint", "http", function, Namespace(), route)
}
