package main

import (
	"context"
	_ "embed"
	"encoding/json"
	"log"
	"regexp"
	"strings"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

//go:embed cloud_events.md
var cloudEventGuide string

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

// Meta can introduce new message types independently of the archive preview.
// Events carry the raw type exactly, including types this worker cannot render,
// so observers' filters on input.message_type match Meta's names.
var cloudMessageTypePattern = regexp.MustCompile(`^[a-z][a-z0-9_]{0,63}$`)

// cloudOnEventsFn is the Cloud API event source: each verified, deduplicated
// message, status or administrative event is one call of it, and its
// observers receive the CloudEvent. It takes no args; observers select events
// with filters on its input fields.
const cloudOnEventsFn = "whatsapp::cloud::on-events"

// emitCloudEvent calls whatsapp::cloud::on-events with event while anything
// observes it and event belongs to the current Cloud account. The call does
// not wait for the observers.
func emitCloudEvent(ctx context.Context, event CloudEvent, observed func() bool, current func(CloudEvent) bool, call func(context.Context, iii.CallRequest) (json.RawMessage, error)) {
	if ctx.Err() != nil || !observed() || !current(event) {
		return
	}
	data, err := json.Marshal(event)
	if err != nil {
		log.Print("Cloud API event serialization failed")
		return
	}
	if _, err := call(ctx, iii.CallRequest{FunctionID: cloudOnEventsFn, Payload: data, Action: iii.VoidAction()}); err != nil && ctx.Err() == nil {
		log.Print("Cloud API event delivery failed")
	}
}

func registerCloudEvents(ctx context.Context, client *iii.Client, manager *Manager) error {
	if _, err := client.RegisterFunction(cloudOnEventsFn, iii.Identity, iii.FunctionOptions{
		Description:    cloudEventDescription(Namespace()),
		RequestFormat:  cloudEventPayloadSchema,
		ResponseFormat: cloudEventPayloadSchema,
		Private:        true,
	}); err != nil {
		return err
	}
	observed := func() bool { return client.IsObserved(cloudOnEventsFn) }
	go func() {
		for {
			select {
			case <-ctx.Done():
				return
			case event, ok := <-manager.CloudEventQueue:
				if !ok {
					return
				}
				emitCloudEvent(ctx, event, observed, manager.cloudEventCurrent, client.Call)
			}
		}
	}()
	return nil
}
