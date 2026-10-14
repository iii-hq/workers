package main

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"reflect"
	"sync/atomic"
	"testing"
)

func cloudCommerceRegistry(t *testing.T, client *cloudClient) *Registry {
	r := cloudActionsRegistry(t, client, cloudTestConfig())
	registerCloudCommerce(r, client)
	registerCloudMarketing(r, client)
	return r
}
func cloudTestBRPayment() map[string]any {
	return map[string]any{"chat_id": "+5511987654321", "text": "Review your order", "reference_id": "invoice-42", "goods_type": "digital-goods", "total_amount": map[string]any{"value": 1000, "offset": 100}, "payment_settings": []any{map[string]any{"type": "payment_link", "payment_link": map[string]any{"uri": "https://pay.example.com/42"}}}}
}
func TestCloudCommerceHTTP(t *testing.T) {
	var last map[string]any
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		if r.Method != "POST" || r.URL.Path != "/v26.0/456/messages" {
			t.Error("unexpected route")
		}
		if err := json.NewDecoder(r.Body).Decode(&last); err != nil {
			t.Error(err)
		}
		fmt.Fprint(w, `{"messages":[{"id":"wamid.commerce"}]}`)
	})
	r := cloudCommerceRegistry(t, client)
	tests := []struct {
		name, kind string
		input      map[string]any
	}{
		{"send_catalog", "catalog_message", map[string]any{"text": "Browse", "thumbnail_product_retailer_id": "sku-1"}},
		{"send_product", "product", map[string]any{"catalog_id": "123", "product_retailer_id": "sku-1"}},
		{"send_products", "product_list", map[string]any{"text": "Choose", "header_text": "Products", "catalog_id": "123", "sections": []any{map[string]any{"title": "Reports", "product_retailer_ids": []any{"sku-1", "sku-2"}}}}},
		{"send_product_carousel", "carousel", map[string]any{"text": "Browse", "catalog_id": "123", "product_retailer_ids": []any{"sku-1", "sku-2"}}},
		{"request_address", "address_message", map[string]any{"text": "Shipping address", "country": "IN", "values": map[string]any{"city": "Mumbai", "in_pin_code": "400063"}}},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			tc.input["chat_id"] = "+919876543210"
			got, err := r.Invoke(context.Background(), "whatsapp::cloud::messages::"+tc.name, tc.input)
			if err != nil {
				t.Fatal(err)
			}
			if got.(CloudSentMessage).Status != "accepted" {
				t.Fatal("not API acceptance")
			}
			i := last["interactive"].(map[string]any)
			if i["type"] != tc.kind {
				t.Fatalf("wrong interactive type: %#v", i)
			}
			if tc.kind == "product" {
				if _, exists := i["body"]; exists {
					t.Fatal("empty optional product body should be omitted")
				}
			}
			if tc.kind == "carousel" {
				cards := i["action"].(map[string]any)["cards"].([]any)
				if len(cards) != 2 || cards[1].(map[string]any)["card_index"] != float64(1) {
					t.Fatal("bad product carousel")
				}
			}
		})
	}
	for _, setting := range []map[string]any{
		{"type": "payment_link", "payment_link": map[string]any{"uri": "https://pay.example.com/42"}},
		{"type": "pix_dynamic_code", "pix_dynamic_code": map[string]any{"code": "000201-test", "merchant_name": "Example", "key": "billing@example.com", "key_type": "EMAIL"}},
		{"type": "boleto", "boleto": map[string]any{"digitable_line": "123456789"}},
		{"type": "offsite_card_pay", "offsite_card_pay": map[string]any{"last_four_digits": "1234", "credential_id": "payment-ref-1"}},
	} {
		input := cloudTestBRPayment()
		input["payment_settings"] = []any{setting}
		_, err := r.Invoke(context.Background(), "whatsapp::cloud::messages::send_order_details", input)
		if err != nil {
			t.Fatal(err)
		}
		params := last["interactive"].(map[string]any)["action"].(map[string]any)["parameters"].(map[string]any)
		if params["payment_type"] != "br" || params["currency"] != "BRL" || !reflect.DeepEqual(params["payment_settings"], []any{setting}) {
			t.Fatalf("invalid payment parameters: %#v", params)
		}
	}
	_, err := r.Invoke(context.Background(), "whatsapp::cloud::messages::send_order_status", map[string]any{"chat_id": "+5511987654321", "text": "Payment confirmed", "reference_id": "invoice-42", "payment": map[string]any{"status": "captured", "timestamp": 1700000000}, "order": map[string]any{"status": "completed"}})
	if err != nil {
		t.Fatal(err)
	}
	if last["interactive"].(map[string]any)["action"].(map[string]any)["name"] != "review_order" {
		t.Fatal("bad order status action")
	}
}
func TestCloudCommerceInvalidBeforeHTTP(t *testing.T) {
	var calls atomic.Int32
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) { calls.Add(1) })
	r := cloudCommerceRegistry(t, client)
	tests := []struct {
		name  string
		input map[string]any
	}{
		{"send_product", map[string]any{"catalog_id": "123", "product_retailer_id": ""}},
		{"send_products", map[string]any{"text": "Products", "catalog_id": "123", "sections": []any{}}},
		{"send_product_carousel", map[string]any{"text": "Browse", "catalog_id": "123", "product_retailer_ids": []any{"one"}}},
		{"request_address", map[string]any{"text": "Address", "country": "BR"}},
		{"send_order_status", map[string]any{"text": "Status", "reference_id": "42", "payment": map[string]any{"status": "refunded"}}},
	}
	for _, tc := range tests {
		tc.input["chat_id"] = "+5511987654321"
		_, err := r.Invoke(context.Background(), "whatsapp::cloud::messages::"+tc.name, tc.input)
		assertCloudDocumentError(t, err, "invalid_request")
	}
	for _, mutate := range []func(map[string]any){
		func(p map[string]any) { p["reference_id"] = "not valid" },
		func(p map[string]any) { p["total_amount"] = map[string]any{"value": -1, "offset": 100} },
		func(p map[string]any) { p["total_amount"] = map[string]any{"value": 1000, "offset": 1} },
		func(p map[string]any) { p["total_amount"] = map[string]any{"value": 1.5, "offset": 100} },
		func(p map[string]any) {
			p["payment_settings"] = []any{map[string]any{"type": "payment_link", "payment_link": map[string]any{"uri": "file:///etc/passwd"}}}
		},
		func(p map[string]any) {
			p["payment_settings"] = []any{map[string]any{"type": "offsite_card_pay", "offsite_card_pay": map[string]any{"last_four_digits": "1234567812345678", "credential_id": "c"}}}
		},
		func(p map[string]any) {
			p["order"] = map[string]any{"items": []any{map[string]any{"retailer_id": "1", "name": "Report", "amount": map[string]any{"value": 500, "offset": 100}, "quantity": 1}}, "subtotal": map[string]any{"value": 1000, "offset": 100}, "tax": map[string]any{"value": 0, "offset": 100}}
		},
	} {
		input := cloudTestBRPayment()
		mutate(input)
		_, err := r.Invoke(context.Background(), "whatsapp::cloud::messages::send_order_details", input)
		assertCloudDocumentError(t, err, "invalid_request")
	}
	if calls.Load() != 0 {
		t.Fatal("invalid commerce request reached Meta")
	}
}

func TestCloudBRItemizedAmounts(t *testing.T) {
	input := cloudTestBRPayment()
	input["total_amount"] = map[string]any{"value": 1950, "offset": 100}
	input["order"] = map[string]any{
		"items":    []any{map[string]any{"retailer_id": "report", "name": "Report", "amount": map[string]any{"value": 1000, "offset": 100}, "sale_amount": map[string]any{"value": 900, "offset": 100}, "quantity": 2}},
		"subtotal": map[string]any{"value": 1800, "offset": 100},
		"tax":      map[string]any{"value": 100, "offset": 100},
		"shipping": map[string]any{"value": 100, "offset": 100},
		"discount": map[string]any{"value": 50, "offset": 100, "description": "Loyalty"},
	}
	var req CloudBROrderDetailsRequest
	if err := decodeCloudRequest(input, &req); err != nil {
		t.Fatal(err)
	}
	body, err := cloudBROrderDetailsPayload(req)
	if err != nil {
		t.Fatal(err)
	}
	params := body["interactive"].(map[string]any)["action"].(map[string]any)["parameters"].(map[string]any)
	if params["order"].(*CloudBROrder).Status != "pending" {
		t.Fatal("new orders must use pending status")
	}
	for _, change := range []func(*CloudBROrderDetailsRequest){
		func(r *CloudBROrderDetailsRequest) { r.TotalAmount.Value++ },
		func(r *CloudBROrderDetailsRequest) { r.Order.Items[0].Quantity = cloudBRMaxAmount },
		func(r *CloudBROrderDetailsRequest) { r.Order.Items[0].SaleAmount.Value = 1000 },
		func(r *CloudBROrderDetailsRequest) { r.Order.Discount.Value = 3000 },
	} {
		var invalid CloudBROrderDetailsRequest
		if err := decodeCloudRequest(input, &invalid); err != nil {
			t.Fatal(err)
		}
		change(&invalid)
		_, err := cloudBROrderDetailsPayload(invalid)
		assertCloudDocumentError(t, err, "invalid_request")
	}
}

func TestCloudBRItemizedSafeIntegerBoundary(t *testing.T) {
	request := CloudBROrderDetailsRequest{
		CloudInteractiveBase: CloudInteractiveBase{ChatID: "+5511987654321", Text: "Order"},
		ReferenceID:          "limit-1", GoodsType: "digital-goods", TotalAmount: CloudBRAmount{Value: cloudBRMaxAmount, Offset: 100},
		Order: &CloudBROrder{Items: []CloudBRItem{{RetailerID: "report", Name: "Report", Amount: CloudBRAmount{Value: cloudBRMaxAmount, Offset: 100}, Quantity: 1}}, Subtotal: CloudBRAmount{Value: cloudBRMaxAmount, Offset: 100}, Tax: CloudBRDescribedAmount{CloudBRAmount: CloudBRAmount{Offset: 100}}},
	}
	if _, err := cloudBROrderDetailsPayload(request); err != nil {
		t.Fatal(err)
	}
	request.Order.Tax.Value = 1
	_, err := cloudBROrderDetailsPayload(request)
	assertCloudDocumentError(t, err, "invalid_request")
	request.Order.Tax.Value = 0
	request.Order.Items[0].Quantity = 2
	_, err = cloudBROrderDetailsPayload(request)
	assertCloudDocumentError(t, err, "invalid_request")
}
