package main

import "encoding/json"

// The request and response format of whatsapp::on-events: the envelope is
// stable while event-specific data intentionally remains open. Observers
// filter on its session_id and kind.
var nativeEventPayloadSchema = json.RawMessage(`{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "Native WhatsApp event",
  "description": "Native session, message, receipt or group event. Data varies by kind and may contain any JSON value. Transport metadata and future envelope fields are allowed.",
  "type": "object",
  "required": ["session_id", "sequence", "kind", "timestamp_ms", "data"],
  "properties": {
    "session_id": {"type": "string"},
    "sequence": {"type": "integer", "minimum": 0},
    "kind": {"type": "string"},
    "timestamp_ms": {"type": "integer"},
    "data": true
  },
  "additionalProperties": true
}`)
