package main

import "encoding/json"

// Discovery contracts preserve the router's existing null and omitted filters.
var nativeEventFilterSchema = json.RawMessage(`{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "Native WhatsApp event filters",
  "description": "Optional filters for whatsapp::events. An omitted, empty or null session_id selects every session. Omitted, empty or null kinds selects every event kind. A null configuration applies no filters.",
  "type": ["object", "null"],
  "additionalProperties": false,
  "properties": {
    "session_id": {"type": ["string", "null"], "pattern": "^[A-Za-z0-9_-]{0,64}$"},
    "kinds": {
      "type": ["array", "null"],
      "maxItems": 128,
      "items": {"type": "string", "pattern": "^[A-Za-z0-9._]{1,128}$"}
    }
  }
}`)

// The envelope is stable while event-specific data intentionally remains open.
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
