package main

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

type schemaTestTag struct{}

func (schemaTestTag) ID() string { return "whatsapp::messages::list" }

func TestEmbeddedCatalogMatchesSource(t *testing.T) {
	raw, err := os.ReadFile("catalog.json")
	if err != nil {
		t.Fatal(err)
	}
	var source []CatalogEntry
	if err := json.Unmarshal(raw, &source); err != nil {
		t.Fatal(err)
	}
	if len(source) != len(catalogEntries) {
		t.Fatal("embedded catalog differs from source; run make generate")
	}
	for _, entry := range source {
		if !reflect.DeepEqual(entry, catalogEntries[entry.ID]) {
			t.Fatalf("embedded contract %s differs from source; run make generate", entry.ID)
		}
	}
}

func TestSDKSchemaAdaptersPreserveContracts(t *testing.T) {
	request, err := iii.InferSchema[wireRequest[schemaTestTag]]()
	if err != nil {
		t.Fatal(err)
	}
	response, err := iii.InferSchema[wireResponse[schemaTestTag]]()
	if err != nil {
		t.Fatal(err)
	}
	for _, tc := range []struct {
		bytes    []byte
		expected map[string]any
	}{{request, catalogEntries[schemaTestTag{}.ID()].RequestFormat}, {response, catalogEntries[schemaTestTag{}.ID()].ResponseFormat}} {
		var got map[string]any
		if err = json.Unmarshal(tc.bytes, &got); err != nil {
			t.Fatal(err)
		}
		for key, want := range tc.expected {
			if !reflect.DeepEqual(got[key], want) {
				t.Errorf("schema %s changed: got %#v want %#v", key, got[key], want)
			}
		}
	}
	b, err := json.Marshal(wireResponse[schemaTestTag]{map[string]any{"messages": []any{}}})
	if err != nil || string(b) != `{"messages":[]}` {
		t.Fatalf("wire response adds wrapper: %s, %v", b, err)
	}
	r := NewRegistry(nil)
	for _, e := range r.Catalog() {
		if typedInstallers[e.ID] == nil {
			t.Fatalf("missing schema installer %s", e.ID)
		}
	}
}
func TestRegistryStrictRequestsAndTransportMetadata(t *testing.T) {
	c := DefaultConfig()
	c.DataDir = filepath.Join(t.TempDir(), "sessions")
	m, err := newTestManager(c)
	if err != nil {
		t.Fatal(err)
	}
	defer m.Shutdown(context.Background())
	r := NewRegistry(m)
	input := map[string]any{"session_id": "test", "_caller_worker_id": "route"}
	if _, err = r.Invoke(context.Background(), "whatsapp::sessions::create", input); err != nil {
		t.Fatal(err)
	}
	if input["_caller_worker_id"] != "route" {
		t.Fatal("request map was mutated")
	}
	for _, input := range []map[string]any{{"session_id": 42}, {"session_id": "other", "unknown": "private-sentinel"}, {}} {
		_, err = r.Invoke(context.Background(), "whatsapp::sessions::create", input)
		var remote *iii.InvocationError
		if !errors.As(err, &remote) || remote.Code != "invalid_request" {
			t.Fatalf("invalid request accepted: %v", err)
		}
	}
	for _, id := range []string{"whatsapp::contacts::remove", "whatsapp::status::send_text", "whatsapp::calls::terminate"} {
		_, err = r.Invoke(context.Background(), id, map[string]any{})
		var remote *iii.InvocationError
		if !errors.As(err, &remote) || remote.Code != "function_not_found" {
			t.Fatalf("unsupported function registered %s: %v", id, err)
		}
	}
}
func TestUIAssetsAreEmbeddedAndAllowlisted(t *testing.T) {
	for _, path := range []string{"whatsapp/page.js", "whatsapp/styles.css"} {
		response, err := uiContent(context.Background(), UIRequest{path})
		if err != nil || len(response.Content) < 100 || response.ContentType == "" {
			t.Fatalf("missing asset %s %v", path, err)
		}
	}
	for _, path := range []string{"../config.yaml", "whatsapp/../page.js", "/whatsapp/page.js", "assets/page.js"} {
		if _, err := uiContent(context.Background(), UIRequest{path}); err == nil {
			t.Fatalf("accepted asset traversal %s", path)
		}
	}
}
