package main

import (
	"bytes"
	"compress/gzip"
	"context"
	_ "embed"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"sort"
	"strconv"
	"strings"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	reflectschema "github.com/invopop/jsonschema"
	validate "github.com/santhosh-tekuri/jsonschema/v6"
	"go.mau.fi/whatsmeow/types"
)

// Generated with the schema adapters so the readable source stays authoritative.
// Compressing this static catalog saves about 470 KB in the release binary.
//
//go:generate go run ./cmd/cataloggen
//go:embed catalog.json.gz
var compressedCatalog []byte

type CatalogEntry struct {
	ID             string         `json:"id"`
	Description    string         `json:"description"`
	RequestFormat  map[string]any `json:"request_format"`
	ResponseFormat map[string]any `json:"response_format"`
}

var catalogEntries = func() map[string]CatalogEntry {
	reader, err := gzip.NewReader(bytes.NewReader(compressedCatalog))
	if err != nil {
		panic(err)
	}
	raw, err := io.ReadAll(reader)
	closeErr := reader.Close()
	if err != nil {
		panic(err)
	}
	if closeErr != nil {
		panic(closeErr)
	}
	var entries []CatalogEntry
	if err := json.Unmarshal(raw, &entries); err != nil {
		panic(err)
	}
	result := make(map[string]CatalogEntry, len(entries))
	for _, e := range entries {
		result[e.ID] = e
	}
	return result
}()

type Handler func(context.Context, map[string]any) (any, error)
type operationHandler struct {
	call   Handler
	schema *validate.Schema
}
type Registry struct {
	Manager  *Manager
	handlers map[string]operationHandler
}

func NewRegistry(manager *Manager) *Registry {
	r := &Registry{Manager: manager, handlers: map[string]operationHandler{}}
	registerSessions(r)
	registerGroups(r)
	registerMessaging(r)
	registerCloud(r)
	registerTunnelOperations(r)
	return r
}

func (r *Registry) Add(id string, handler func(context.Context, map[string]any) (any, error)) {
	entry, ok := catalogEntries[id]
	if !ok {
		panic("missing catalog entry: " + id)
	}
	if _, exists := r.handlers[id]; exists {
		panic("duplicate handler: " + id)
	}
	compiler := validate.NewCompiler()
	uri := "https://whatsapp.invalid/request.json"
	if err := compiler.AddResource(uri, entry.RequestFormat); err != nil {
		panic(err)
	}
	schema, err := compiler.Compile(uri)
	if err != nil {
		panic(fmt.Errorf("%s: %w", id, err))
	}
	r.handlers[id] = operationHandler{handler, schema}
}

func (r *Registry) Invoke(ctx context.Context, id string, input map[string]any) (any, error) {
	h, ok := r.handlers[id]
	if !ok {
		return nil, &iii.InvocationError{Code: "function_not_found", Message: "WhatsApp operation is not supported"}
	}
	if input == nil {
		input = map[string]any{}
	}
	clean := make(map[string]any, len(input))
	for k, v := range input {
		if k != "_caller_worker_id" {
			clean[k] = v
		}
	}
	if err := h.schema.Validate(clean); err != nil {
		return nil, &iii.InvocationError{Code: "invalid_request", Message: "Request does not match the function schema"}
	}
	if r.Manager == nil {
		return nil, errors.New("worker is not initialized")
	}
	started := time.Now()
	configCtx, configCancel := context.WithTimeout(ctx, time.Duration(DefaultConfig().OperationTimeoutSecs)*time.Second)
	config, err := r.Manager.ConfigContext(configCtx)
	configCancel()
	if err != nil {
		return nil, err
	}
	ctx, cancel := context.WithDeadline(ctx, started.Add(time.Duration(config.OperationTimeoutSecs)*time.Second))
	defer cancel()
	// A lifecycle transition drains active operations before closing their stores.
	if !strings.HasPrefix(id, "whatsapp::sessions::") && stringArg(clean, "session_id") != "" {
		s, err := r.Manager.sessionContext(ctx, stringArg(clean, "session_id"))
		if err != nil {
			return nil, err
		}
		if err := lockSessionContext(ctx, contextReadMutex{&s.operations}); err != nil {
			return nil, err
		}
		defer s.operations.RUnlock()
		current, err := r.Manager.sessionContext(ctx, stringArg(clean, "session_id"))
		if err != nil {
			return nil, err
		}
		if current != s {
			return nil, errors.New("session changed during the operation; refresh its state")
		}
	}
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	return h.call(ctx, clean)
}

func (r *Registry) Catalog() []CatalogEntry {
	entries := make([]CatalogEntry, 0, len(r.handlers))
	for id := range r.handlers {
		entries = append(entries, catalogEntries[id])
	}
	sort.Slice(entries, func(i, j int) bool { return entries[i].ID < entries[j].ID })
	return entries
}
func (r *Registry) Install(client *iii.Client) error {
	for _, e := range r.Catalog() {
		if err := typedInstallers[e.ID](client, r); err != nil {
			return err
		}
	}
	return nil
}

// The SDK accepts schemas through typed reflection. Generated zero-sized tags
// select immutable contracts, including nullable fields from the existing API.
type operation interface{ ID() string }
type wireRequest[T operation] map[string]any

func (wireRequest[T]) JSONSchema() *reflectschema.Schema {
	var t T
	return &reflectschema.Schema{Extras: catalogEntries[t.ID()].RequestFormat}
}

type wireResponse[T operation] struct{ Value any }

func (w wireResponse[T]) MarshalJSON() ([]byte, error) { return json.Marshal(w.Value) }
func (wireResponse[T]) JSONSchema() *reflectschema.Schema {
	var t T
	return &reflectschema.Schema{Extras: catalogEntries[t.ID()].ResponseFormat}
}
func installOperation[T operation](client *iii.Client, r *Registry) error {
	var t T
	metadata, _ := json.Marshal(map[string]any{"description": catalogEntries[t.ID()].Description})
	return iii.RegisterFunctionTyped[wireRequest[T], wireResponse[T]](client, t.ID(), func(ctx context.Context, input wireRequest[T]) (wireResponse[T], error) {
		result, err := r.Invoke(ctx, t.ID(), map[string]any(input))
		return wireResponse[T]{result}, err
	}, iii.RegisterFunctionOptions{Metadata: metadata})
}

func stringArg(v map[string]any, key string) string { s, _ := v[key].(string); return s }
func boolArg(v map[string]any, key string) bool     { b, _ := v[key].(bool); return b }
func intArg(v map[string]any, key string, fallback int) int {
	switch n := v[key].(type) {
	case float64:
		return int(n)
	case int:
		return n
	case json.Number:
		i, e := n.Int64()
		if e == nil {
			return int(i)
		}
	}
	return fallback
}
func stringsArg(v map[string]any, key string) []string {
	if a, ok := v[key].([]string); ok {
		return a
	}
	a, _ := v[key].([]any)
	out := make([]string, 0, len(a))
	for _, x := range a {
		if s, ok := x.(string); ok {
			out = append(out, s)
		}
	}
	return out
}
func applied() map[string]any { return map[string]any{"applied": true} }
func parseJID(input string) (types.JID, error) {
	if len(input) == 0 || len(input) > 128 {
		return types.EmptyJID, errors.New("invalid JID length")
	}
	digits := func(value string) bool {
		if value == "" {
			return false
		}
		for _, c := range []byte(value) {
			if c < '0' || c > '9' {
				return false
			}
		}
		return true
	}
	user, server, hasServer := strings.Cut(input, "@")
	if !hasServer {
		phone := strings.TrimPrefix(input, "+")
		if len(phone) < 7 || len(phone) > 15 || !digits(phone) {
			return types.EmptyJID, errors.New("phone must contain 7..15 digits")
		}
		return types.NewJID(phone, types.DefaultUserServer), nil
	}
	valid := false
	switch server {
	case types.LegacyUserServer, types.DefaultUserServer, types.HiddenUserServer:
		identity, device, hasDevice := strings.Cut(user, ":")
		valid = digits(identity)
		if hasDevice {
			_, err := strconv.ParseUint(device, 10, 16)
			valid = valid && digits(device) && err == nil
		}
		if server == types.LegacyUserServer {
			server = types.DefaultUserServer
		}
	case types.GroupServer:
		parts := strings.Split(user, "-")
		valid = len(parts) <= 2
		for _, part := range parts {
			valid = valid && digits(part)
		}
	case types.BroadcastServer:
		valid = user == "status" || digits(user)
	case types.NewsletterServer:
		valid = digits(user)
	default:
		return types.EmptyJID, errors.New("unsupported JID server")
	}
	if !valid {
		return types.EmptyJID, errors.New("invalid JID user")
	}
	return types.ParseJID(user + "@" + server)
}
func decodeMedia(value string, max int) ([]byte, error) {
	if len(value) > base64.StdEncoding.EncodedLen(max) {
		return nil, errors.New("media exceeds max_media_bytes")
	}
	b, err := base64.StdEncoding.Strict().DecodeString(value)
	if err != nil {
		return nil, errors.New("invalid base64 media")
	}
	if len(b) == 0 || len(b) > max {
		return nil, errors.New("media must be nonempty and within max_media_bytes")
	}
	return b, nil
}
