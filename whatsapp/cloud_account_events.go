package main

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"maps"
	"strconv"
	"strings"
)

func cloudAccountEvents(config CloudConfig, entry, change map[string]any, field string) ([]CloudEvent, error) {
	value, err := cloudObject(change["value"])
	if err != nil {
		return nil, err
	}
	key := ""
	if strings.HasPrefix(field, "group_") {
		key = "groups"
	} else if field == "calls" {
		key = "calls"
	}
	if key != "" && value[key] != nil {
		items, err := cloudArray(value[key])
		if err != nil || len(items) > 1000 {
			return nil, fmt.Errorf("invalid administrative event collection")
		}
		out := make([]CloudEvent, 0, len(items))
		for _, item := range items {
			copy := maps.Clone(value)
			copy[key] = []any{item}
			event, err := cloudAccountEvent(config, entry, copy, field)
			if err != nil {
				return nil, err
			}
			if event != nil {
				out = append(out, *event)
			}
		}
		return out, nil
	}
	event, err := cloudAccountEvent(config, entry, value, field)
	if err != nil || event == nil {
		return nil, err
	}
	return []CloudEvent{*event}, nil
}

// Administrative payloads do not necessarily identify a phone or conversation.
// Keep those identities empty instead of attributing a WABA-wide event to a phone.
func cloudAccountEvent(config CloudConfig, entry, value map[string]any, field string) (*CloudEvent, error) {
	if !cloudMessageTypePattern.MatchString(field) {
		return nil, fmt.Errorf("invalid webhook field")
	}
	phone := cloudString(cloudNested(value, "metadata"), "phone_number_id")
	if phone == "" {
		phone = cloudString(value, "phone_number_id")
	}
	if phone != "" && phone != config.PhoneNumberID {
		return nil, nil
	}
	seconds := int64(0)
	switch stamp := entry["time"].(type) {
	case float64:
		if stamp >= 0 && stamp < 9e12 {
			seconds = int64(stamp)
		}
	case string:
		if parsed, err := strconv.ParseInt(stamp, 10, 64); err == nil && parsed >= 0 && parsed < 9e12 {
			seconds = parsed
		}
	}
	// Hash canonical JSON, not incoming whitespace. Include entry time so distinct
	// occurrences of the same administrative state are not merged when provided.
	canonical, _ := json.Marshal([]any{config.BusinessAccountID, phone, field, entry["time"], value})
	hash := sha256.Sum256(canonical)
	data, _ := json.Marshal(cloudSanitizeResult(Config{Cloud: config}, value))
	event := CloudEvent{Kind: "webhook.received", EventID: hex.EncodeToString(hash[:]), Field: field, Event: cloudString(value, "event"), BusinessAccountID: config.BusinessAccountID, PhoneNumberID: phone, TimestampMS: seconds * 1000, Data: data}
	for _, key := range []string{"groups", "calls"} {
		items, _ := value[key].([]any)
		if len(items) == 1 {
			item, ok := items[0].(map[string]any)
			if !ok {
				return nil, fmt.Errorf("invalid administrative event")
			}
			if kind := cloudString(item, "type"); kind != "" {
				event.Event = kind
			}
			if kind := cloudString(item, "event"); kind != "" {
				event.Event = kind
			}
			if group := cloudString(item, "group_id"); group != "" {
				if !cloudGroupIDPattern.MatchString(group) {
					return nil, fmt.Errorf("invalid Cloud group ID")
				}
				event.GroupID = group
			}
			if timestamp, err := strconv.ParseInt(cloudString(item, "timestamp"), 10, 64); err == nil && timestamp >= 0 && timestamp < 9e12 {
				event.TimestampMS = timestamp * 1000
			}
		}
	}
	if len(event.Event) > 128 {
		event.Event = ""
	}
	event = boundedCloudEvent(event)
	return &event, nil
}
