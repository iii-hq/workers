package main

import (
	"bytes"
	"context"
	_ "embed"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"regexp"
	"slices"
	"strings"
	"sync"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

//go:embed cloud_events.md
var cloudEventGuide string

//go:embed cloud_events.config.schema.json
var cloudEventConfigSchema []byte

//go:embed cloud_events.payload.schema.json
var cloudEventPayloadSchema []byte

func cloudEventDescription(namespace string) string {
	encoded, _ := json.Marshal(namespace)
	return strings.ReplaceAll(cloudEventGuide, `"<provider-namespace>"`, string(encoded))
}

// CloudEvent contains one verified Meta message, status or administrative event.
// Chat and sender IDs use the same public format as cloud::chats::list.
type CloudEvent struct {
	Kind              string          `json:"kind"`
	BusinessAccountID string          `json:"business_account_id"`
	PhoneNumberID     string          `json:"phone_number_id"`
	MessageID         string          `json:"message_id"`
	ChatID            string          `json:"chat_id"`
	SenderID          string          `json:"sender_id,omitempty"`
	MessageType       string          `json:"message_type,omitempty"`
	Text              string          `json:"text,omitempty"`
	Status            string          `json:"status,omitempty"`
	TimestampMS       int64           `json:"timestamp_ms"`
	Data              json.RawMessage `json:"data"`
	EventID           string          `json:"event_id,omitempty"`
	Field             string          `json:"field,omitempty"`
	Event             string          `json:"event,omitempty"`
	GroupID           string          `json:"group_id,omitempty"`
	ParticipantID     string          `json:"participant_id,omitempty"`
	epoch             uint64
}

type CloudEventFilter struct {
	Kinds             []string `json:"kinds"`
	ChatIDs           []string `json:"chat_ids"`
	SenderIDs         []string `json:"sender_ids"`
	MessageTypes      []string `json:"message_types"`
	Statuses          []string `json:"statuses"`
	BusinessAccountID string   `json:"business_account_id"`
	PhoneNumberID     string   `json:"phone_number_id"`
	Fields            []string `json:"fields"`
	Events            []string `json:"events"`
	GroupIDs          []string `json:"group_ids"`
	ParticipantIDs    []string `json:"participant_ids"`
}

type cloudEventBinding struct {
	filter CloudEventFilter
	config iii.TriggerConfig
}

type CloudEventRouter struct {
	mu       sync.RWMutex
	bindings map[string]cloudEventBinding
}

// Meta can introduce new message types independently of the archive preview.
// Filters match the raw type exactly, including types this worker cannot render.
var cloudMessageTypePattern = regexp.MustCompile(`^[a-z][a-z0-9_]{0,63}$`)

func parseCloudEventFilter(raw json.RawMessage) (CloudEventFilter, error) {
	var filter CloudEventFilter
	if len(raw) > 64*1024 {
		return filter, errors.New("Cloud API event filter must be at most 64 KiB")
	}
	trimmed := bytes.TrimSpace(raw)
	if len(trimmed) == 0 || trimmed[0] != '{' {
		return filter, errors.New("Cloud API event filter must be a JSON object")
	}
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&filter); err != nil {
		return filter, fmt.Errorf("invalid Cloud API event filter: %w", err)
	}
	var trailing any
	if err := decoder.Decode(&trailing); err != io.EOF {
		return filter, errors.New("Cloud API event filter must contain exactly one JSON object")
	}
	for _, list := range []struct {
		name   string
		values []string
	}{
		{"kinds", filter.Kinds}, {"chat_ids", filter.ChatIDs}, {"sender_ids", filter.SenderIDs},
		{"message_types", filter.MessageTypes}, {"statuses", filter.Statuses},
		{"fields", filter.Fields}, {"events", filter.Events},
		{"group_ids", filter.GroupIDs}, {"participant_ids", filter.ParticipantIDs},
	} {
		if len(list.values) > 128 {
			return filter, fmt.Errorf("%s must contain at most 128 values", list.name)
		}
	}
	if len(filter.Kinds) == 0 {
		filter.Kinds = []string{"message.received"}
	}
	for _, kind := range filter.Kinds {
		if kind != "message.received" && kind != "message.status" && kind != "webhook.received" {
			return filter, errors.New("kinds must contain only message.received, message.status or webhook.received")
		}
	}
	for _, identities := range []struct {
		name   string
		values []string
	}{{"chat_ids", filter.ChatIDs}, {"sender_ids", filter.SenderIDs}, {"participant_ids", filter.ParticipantIDs}} {
		for i, raw := range identities.values {
			id, err := normalizeCloudChatID(raw)
			if identities.name == "chat_ids" {
				id, err = normalizeCloudConversationID(raw)
			}
			if err != nil {
				return filter, fmt.Errorf("invalid %s[%d]: %w", identities.name, i, err)
			}
			identities.values[i] = id
		}
	}
	for _, id := range filter.GroupIDs {
		if !cloudGroupIDPattern.MatchString(id) {
			return filter, errors.New("group_ids must contain raw Cloud group IDs, without native JID or group: prefixes")
		}
	}
	for _, kind := range filter.MessageTypes {
		if !cloudMessageTypePattern.MatchString(kind) {
			return filter, errors.New("message_types must contain raw Meta type names of 1..64 lowercase letters, digits or underscores, starting with a letter")
		}
	}
	for _, field := range filter.Fields {
		if !cloudMessageTypePattern.MatchString(field) {
			return filter, errors.New("fields must contain raw Meta webhook field names")
		}
	}
	for _, event := range filter.Events {
		if len(event) == 0 || len(event) > 128 || strings.TrimSpace(event) != event {
			return filter, errors.New("events must contain event names of 1..128 bytes without surrounding whitespace")
		}
	}
	for _, status := range filter.Statuses {
		if status != "sent" && status != "delivered" && status != "read" && status != "failed" {
			return filter, errors.New("statuses must contain only sent, delivered, read or failed")
		}
	}
	for _, id := range []struct{ name, value string }{
		{"business_account_id", filter.BusinessAccountID}, {"phone_number_id", filter.PhoneNumberID},
	} {
		if id.value != "" && !cloudMetaIDPattern.MatchString(id.value) {
			return filter, fmt.Errorf("%s must be a numeric Meta identifier of at most 32 digits", id.name)
		}
	}
	return filter, nil
}

func (filter CloudEventFilter) matches(event CloudEvent) bool {
	return slices.Contains(filter.Kinds, event.Kind) &&
		(len(filter.ChatIDs) == 0 || slices.Contains(filter.ChatIDs, event.ChatID)) &&
		(len(filter.SenderIDs) == 0 || slices.Contains(filter.SenderIDs, event.SenderID)) &&
		(len(filter.MessageTypes) == 0 || slices.Contains(filter.MessageTypes, event.MessageType)) &&
		(len(filter.Statuses) == 0 || slices.Contains(filter.Statuses, event.Status)) &&
		(len(filter.Fields) == 0 || slices.Contains(filter.Fields, event.Field)) &&
		(len(filter.Events) == 0 || slices.Contains(filter.Events, event.Event)) &&
		(len(filter.GroupIDs) == 0 || slices.Contains(filter.GroupIDs, event.GroupID)) &&
		(len(filter.ParticipantIDs) == 0 || slices.Contains(filter.ParticipantIDs, event.ParticipantID)) &&
		(filter.BusinessAccountID == "" || filter.BusinessAccountID == event.BusinessAccountID) &&
		(filter.PhoneNumberID == "" || filter.PhoneNumberID == event.PhoneNumberID)
}

func (r *CloudEventRouter) RegisterTrigger(_ context.Context, config iii.TriggerConfig) error {
	filter, err := parseCloudEventFilter(config.Config)
	if err != nil {
		return err
	}
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.bindings == nil {
		r.bindings = map[string]cloudEventBinding{}
	}
	if _, exists := r.bindings[config.ID]; !exists && len(r.bindings) >= 1024 {
		return errors.New("Cloud API event subscription limit reached (1024)")
	}
	r.bindings[config.ID] = cloudEventBinding{filter: filter, config: config}
	return nil
}

func (r *CloudEventRouter) UnregisterTrigger(_ context.Context, config iii.TriggerConfig) error {
	r.mu.Lock()
	defer r.mu.Unlock()
	delete(r.bindings, config.ID)
	return nil
}

func (r *CloudEventRouter) deliver(ctx context.Context, event CloudEvent, current func(CloudEvent) bool, trigger func(context.Context, iii.TriggerRequest) (json.RawMessage, error)) {
	if ctx.Err() != nil || !current(event) {
		return
	}
	r.mu.RLock()
	bindings := []iii.TriggerConfig{}
	for _, binding := range r.bindings {
		if binding.filter.matches(event) {
			bindings = append(bindings, binding.config)
		}
	}
	r.mu.RUnlock()
	if len(bindings) == 0 {
		return
	}
	data, err := json.Marshal(event)
	if err != nil {
		log.Print("Cloud API event serialization failed")
		return
	}
	for _, binding := range bindings {
		if ctx.Err() != nil || !current(event) {
			return
		}
		callCtx, cancel := context.WithTimeout(ctx, 5*time.Second)
		_, err := trigger(callCtx, iii.TriggerRequest{
			FunctionID: binding.FunctionID, Data: data, Namespace: binding.Namespace,
			Metadata: binding.Metadata, Action: iii.VoidAction(), Timeout: 5 * time.Second,
		})
		cancel()
		if err != nil && ctx.Err() == nil {
			log.Print("Cloud API event delivery failed")
		}
	}
}

func registerCloudEvents(ctx context.Context, client *iii.Client, manager *Manager) error {
	router := &CloudEventRouter{}
	if err := client.RegisterTriggerType("whatsapp::cloud::events", cloudEventDescription(Namespace()), router, iii.RegisterTriggerTypeOptions{
		TriggerRequestFormat: cloudEventConfigSchema,
		CallRequestFormat:    cloudEventPayloadSchema,
	}); err != nil {
		return err
	}
	go func() {
		for {
			select {
			case <-ctx.Done():
				return
			case event, ok := <-manager.CloudEventQueue:
				if !ok {
					return
				}
				router.deliver(ctx, event, manager.cloudEventCurrent, client.Trigger)
			}
		}
	}()
	return nil
}
