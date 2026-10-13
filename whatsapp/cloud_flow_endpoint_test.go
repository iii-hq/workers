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
	"strings"
	"testing"
)

func flowTestKey(t *testing.T) (*rsa.PrivateKey, string) {
	t.Helper()
	key, err := rsa.GenerateKey(rand.Reader, 2048)
	if err != nil {
		t.Fatal(err)
	}
	return key, string(pem.EncodeToMemory(&pem.Block{Type: "RSA PRIVATE KEY", Bytes: x509.MarshalPKCS1PrivateKey(key)}))
}

func flowTestEnvelope(t *testing.T, public *rsa.PublicKey, payload string) ([]byte, []byte, []byte) {
	t.Helper()
	key := make([]byte, 16)
	iv := make([]byte, 16)
	_, _ = rand.Read(key)
	_, _ = rand.Read(iv)
	encryptedKey, err := rsa.EncryptOAEP(sha256.New(), rand.Reader, public, key, nil)
	if err != nil {
		t.Fatal(err)
	}
	block, _ := aes.NewCipher(key)
	gcm, _ := cipher.NewGCMWithNonceSize(block, len(iv))
	encoded, _ := json.Marshal(map[string]string{"encrypted_aes_key": base64.StdEncoding.EncodeToString(encryptedKey), "initial_vector": base64.StdEncoding.EncodeToString(iv), "encrypted_flow_data": base64.StdEncoding.EncodeToString(gcm.Seal(nil, iv, []byte(payload), nil))})
	return encoded, key, iv
}

// flowTestOpen decrypts a Flow response, which is sealed under the request IV with every bit flipped.
func flowTestOpen(t *testing.T, key, iv []byte, body string) string {
	t.Helper()
	encrypted, err := base64.StdEncoding.DecodeString(body)
	if err != nil {
		t.Fatal(err)
	}
	block, _ := aes.NewCipher(key)
	gcm, _ := cipher.NewGCMWithNonceSize(block, len(iv))
	for i := range iv {
		iv[i] ^= 0xff
	}
	plain, err := gcm.Open(nil, iv, encrypted, nil)
	if err != nil {
		t.Fatal(err)
	}
	return string(plain)
}

func TestCloudFlowEndpointSignedEncryptedExchange(t *testing.T) {
	key, privateKey := flowTestKey(t)
	config := cloudTestConfig()
	config.Cloud.FlowPrivateKey = privateKey
	config.Cloud.FlowHandlerFunction = "orders::flow"
	for _, action := range []string{"ping", "INIT"} {
		raw, aesKey, iv := flowTestEnvelope(t, &key.PublicKey, `{"version":"3.0","action":"`+action+`","flow_token":"session123"}`)
		calls := 0
		handler := func(_ context.Context, payload json.RawMessage) (json.RawMessage, error) {
			calls++
			return json.RawMessage(`{"screen":"REPORT","data":{"ready":true}}`), nil
		}
		response := processCloudFlow(context.Background(), config, raw, cloudTestSignature(raw), handler)
		if response.StatusCode != 200 {
			t.Fatalf("flow: %+v", response)
		}
		plain := flowTestOpen(t, aesKey, iv, response.Body)
		if action == "ping" && (calls != 0 || !strings.Contains(plain, "active")) {
			t.Fatal("ping did not stay local")
		}
		if action == "INIT" && (calls != 1 || !strings.Contains(plain, "REPORT")) {
			t.Fatal("handler not dispatched exactly once")
		}
		invalid := processCloudFlow(context.Background(), config, raw, "sha256="+strings.Repeat("0", 64), handler)
		if invalid.StatusCode != 403 {
			t.Fatal("unsigned envelope accepted")
		}
	}
	redacted, _ := json.Marshal(config.Redacted())
	if strings.Contains(string(redacted), "PRIVATE KEY") {
		t.Fatal("private key exposed")
	}
}

func TestCloudFlowEndpointUnconfiguredAndCorruptData(t *testing.T) {
	config := cloudTestConfig()
	if response := processCloudFlow(context.Background(), config, []byte(`{}`), "", nil); response.StatusCode != 503 {
		t.Fatal("missing key accepted")
	}
	_, config.Cloud.FlowPrivateKey = flowTestKey(t)
	raw := []byte(`{"encrypted_aes_key":"bad","encrypted_flow_data":"bad","initial_vector":"bad"}`)
	response := processCloudFlow(context.Background(), config, raw, cloudTestSignature(raw), nil)
	if response.StatusCode != 421 || strings.Contains(response.Body, "PRIVATE KEY") {
		t.Fatalf("invalid encrypted request: %+v", response)
	}
}

func TestCloudFlowErrorNotificationAcknowledgedWithoutBusinessReplay(t *testing.T) {
	key, privateKey := flowTestKey(t)
	config := cloudTestConfig()
	config.Cloud.FlowPrivateKey = privateKey
	for _, action := range []string{"INIT", "data_exchange"} {
		raw, aesKey, iv := flowTestEnvelope(t, &key.PublicKey, `{"version":"3.0","action":"`+action+`","flow_token":"session123","data":{"error":"invalid-screen","error_message":"Unable to render"}}`)
		calls := 0
		result := processCloudFlow(context.Background(), config, raw, cloudTestSignature(raw), func(context.Context, json.RawMessage) (json.RawMessage, error) { calls++; return nil, nil })
		if result.StatusCode != 200 || calls != 0 {
			t.Fatalf("invalid error acknowledgement: status=%d business calls=%d", result.StatusCode, calls)
		}
		if flowTestOpen(t, aesKey, iv, result.Body) != `{"data":{"acknowledged":true}}` {
			t.Fatal("Flow error acknowledgement did not follow encrypted protocol")
		}
	}
}
