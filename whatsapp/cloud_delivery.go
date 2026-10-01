package main

import (
	"encoding/json"
	"errors"
	"log"
)

func boundedCloudEvent(event CloudEvent) CloudEvent {
	raw, _ := json.Marshal(event)
	if len(raw) > 64*1024 {
		event.Data = json.RawMessage(`{"content_omitted":true}`)
		// JSON escaping can expand each text byte to six bytes.
		event.Text = clipInboxText(event.Text, 8192)
	}
	return event
}

// Store first, then enqueue only newly committed events. Callback delivery is
// best effort; the archive and dedup markers remain durable independently.
func (m *Manager) storeCloudDelivery(config Config, batch cloudDeliveryBatch) error {
	inbox, err := m.CloudInbox(config)
	if err != nil {
		return err
	}
	m.mu.RLock()
	defer m.mu.RUnlock()
	if m.closed || !m.config.Cloud.Enabled || m.config.Cloud != config.Cloud {
		return errors.New("Cloud configuration is disabled or changed")
	}
	events, err := inbox.RecordCloudDelivery(batch.messages, batch.receipts, batch.events)
	if err != nil {
		return err
	}
	dropped := false
	for _, event := range events {
		event.epoch = m.cloudEpoch
		select {
		case m.CloudEventQueue <- event:
		default:
			dropped = true
		}
	}
	if dropped {
		log.Print("Cloud event queue is full; reconcile with the Cloud archive")
	}
	return nil
}

func (m *Manager) cloudEventCurrent(event CloudEvent) bool {
	m.mu.RLock()
	defer m.mu.RUnlock()
	return !m.closed && m.config.Cloud.Enabled && event.epoch == m.cloudEpoch &&
		event.BusinessAccountID == m.config.Cloud.BusinessAccountID && event.PhoneNumberID == m.config.Cloud.PhoneNumberID
}
