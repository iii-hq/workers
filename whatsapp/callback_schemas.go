package main

import (
	"encoding/json"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	reflectschema "github.com/invopop/jsonschema"
)

// Preserve the HTTP callbacks' own decoding and HTTP 400 responses. The typed
// SDK would otherwise turn a malformed field into an invocation error before
// the handler can return its established HTTP response.
type callbackRequest[T any] struct {
	Raw json.RawMessage
}

func (r *callbackRequest[T]) UnmarshalJSON(data []byte) error {
	r.Raw = append(r.Raw[:0], data...)
	return nil
}

func (callbackRequest[T]) JSONSchema() *reflectschema.Schema {
	reflector := reflectschema.Reflector{
		DoNotReference:             true,
		AllowAdditionalProperties:  true,
		RequiredFromJSONSchemaTags: true,
	}
	var value T
	schema := reflector.Reflect(value)
	// HTTP request fields are optional: handlers return an HTTP error for an
	// absent body or verification query. Transport metadata and future fields
	// remain extensible, and JSON null follows the existing struct decoder.
	if schema.Type == "object" {
		if schema.Properties != nil {
			for pair := schema.Properties.Oldest(); pair != nil; pair = pair.Next() {
				pair.Value = &reflectschema.Schema{AnyOf: []*reflectschema.Schema{pair.Value, {Type: "null"}}}
			}
		}
		return &reflectschema.Schema{AnyOf: []*reflectschema.Schema{schema, {Type: "null"}}}
	}
	return schema
}

// These callbacks ignore their payload and reconcile authoritative state. Keep
// every JSON value accepted by the previous handlers; notifications may gain
// fields without requiring a new worker release.
type callbackNotification struct{}

func (callbackNotification) JSONSchema() *reflectschema.Schema {
	return &reflectschema.Schema{
		Description: "Payload is ignored; the callback reads authoritative state instead.",
		AnyOf: []*reflectschema.Schema{
			{Type: "object", AdditionalProperties: reflectschema.TrueSchema},
			{Type: "array", Items: reflectschema.TrueSchema},
			{Type: "string"}, {Type: "number"}, {Type: "boolean"}, {Type: "null"},
		},
	}
}

type callbackConfigurationID struct {
	ID string `json:"id"`
}

// secretChangedNotification is the part of a secrets::on-changed delivery the
// worker reads; the rest (name, action, fingerprint, updated_at) is ignored.
type secretChangedNotification struct {
	Ref string `json:"ref,omitempty"`
}

// observedSecretChanged is what whatsapp::on-secret-changed receives from its
// capture: [output] observer: the Observed envelope, with the secrets::on-changed
// event as its output.
type observedSecretChanged struct {
	Output secretChangedNotification `json:"output"`
}

type callbackOK struct {
	OK bool `json:"ok"`
}

type callbackAccepted struct {
	Accepted bool `json:"accepted"`
}

func interfaceCallbackError() error {
	return &iii.InvocationError{Code: "invalid_request", Message: "Interface capture does not execute callbacks"}
}
