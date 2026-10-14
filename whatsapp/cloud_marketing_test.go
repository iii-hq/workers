package main

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"sync/atomic"
	"testing"
)

func TestCloudMarketingAndDirectHTTP(t *testing.T) {
	var path string
	var last map[string]any
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		path = r.URL.Path
		if r.Header.Get("Authorization") != "Bearer test-access-secret" {
			t.Error("missing auth")
		}
		if err := json.NewDecoder(r.Body).Decode(&last); err != nil {
			t.Error(err)
		}
		fmt.Fprint(w, `{"messages":[{"id":"wamid.marketing"}]}`)
	})
	r := cloudCommerceRegistry(t, client)
	_, err := r.Invoke(context.Background(), "whatsapp::cloud::messages::send_marketing_template", map[string]any{"chat_id": "+15551234567", "name": "promotion", "language": "en_US", "message_activity_sharing": false})
	if err != nil {
		t.Fatal(err)
	}
	if path != "/v26.0/456/marketing_messages" || last["product_policy"] != "STRICT" || last["message_activity_sharing"] != false {
		t.Fatalf("wrong marketing route or defaults: %s %#v", path, last)
	}
	_, err = r.Invoke(context.Background(), "whatsapp::cloud::messages::send_direct", map[string]any{"chat_id": "+15551234567", "category": "utility", "type": "text", "text": "Order shipped", "ttl_seconds": 600, "template_name": "order_shipped"})
	if err != nil {
		t.Fatal(err)
	}
	if path != "/v26.0/456/messages" || last["category"] != "utility" || last["ttl_seconds"] != float64(600) || last["direct_send_config"].(map[string]any)["template_name"] != "order_shipped" {
		t.Fatalf("bad direct payload: %#v", last)
	}
}
func TestCloudMarketingInvalidBeforeHTTP(t *testing.T) {
	var calls atomic.Int32
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) { calls.Add(1) })
	r := cloudCommerceRegistry(t, client)
	for _, input := range []map[string]any{
		{"chat_id": "+15551234567", "category": "marketing", "type": "text", "text": "Hi"},
		{"chat_id": "US.abc123", "category": "authentication", "type": "text", "text": "Code"},
		{"chat_id": "+15551234567", "category": "authentication", "type": "text", "text": "Code", "ttl_seconds": 901},
		{"chat_id": "+15551234567", "category": "utility", "type": "text", "text": "Hi", "ttl_seconds": 29},
		{"chat_id": "+15551234567", "category": "service", "type": "text", "text": "Hi", "ttl_seconds": 600},
		{"chat_id": "+15551234567", "category": "authentication", "type": "text", "text": "Code", "template_name": "not_allowed"},
	} {
		_, err := r.Invoke(context.Background(), "whatsapp::cloud::messages::send_direct", input)
		assertCloudDocumentError(t, err, "invalid_request")
	}
	_, err := r.Invoke(context.Background(), "whatsapp::cloud::messages::send_marketing_template", map[string]any{"chat_id": "+15551234567", "name": "promotion", "language": "en_US", "product_policy": "unknown"})
	assertCloudDocumentError(t, err, "invalid_request")
	if calls.Load() != 0 {
		t.Fatal("invalid marketing request reached Meta")
	}
}
func TestCloudMarketingErrorNoRetry(t *testing.T) {
	var calls atomic.Int32
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		w.WriteHeader(400)
		fmt.Fprint(w, `{"error":{"code":100,"message":"test-access-secret"}}`)
	})
	r := cloudCommerceRegistry(t, client)
	_, err := r.Invoke(context.Background(), "whatsapp::cloud::messages::send_marketing_template", map[string]any{"chat_id": "+15551234567", "name": "promotion", "language": "en_US"})
	assertCloudDocumentError(t, err, "cloud_api_error")
	if calls.Load() != 1 {
		t.Fatal("marketing was retried")
	}
}

func TestCloudMarketingAndCommerceAmbiguousDispatch(t *testing.T) {
	for _, name := range []string{"send_order_details", "send_marketing_template"} {
		t.Run(name, func(t *testing.T) {
			ctx, cancel := context.WithCancel(context.Background())
			defer cancel()
			var calls atomic.Int32
			client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
				calls.Add(1)
				cancel()
				w.WriteHeader(http.StatusAccepted)
			})
			r := cloudCommerceRegistry(t, client)
			input := cloudTestBRPayment()
			if name == "send_marketing_template" {
				input = map[string]any{"chat_id": "+15551234567", "name": "promotion", "language": "en_US"}
			}
			_, err := r.Invoke(ctx, "whatsapp::cloud::messages::"+name, input)
			assertCloudDocumentError(t, err, "cloud_outcome_unknown")
			if calls.Load() != 1 {
				t.Fatalf("dispatched %d requests; must not retry", calls.Load())
			}
		})
	}
}
