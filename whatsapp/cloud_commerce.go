package main

import (
	"encoding/json"
	"strconv"
	"strings"
	"time"
)

type CloudCatalogRequest struct {
	CloudInteractiveBase
	ThumbnailProductRetailerID string `json:"thumbnail_product_retailer_id,omitempty"`
}
type CloudProductRequest struct {
	CloudInteractiveBase
	CatalogID         string `json:"catalog_id"`
	ProductRetailerID string `json:"product_retailer_id"`
}
type CloudProductSection struct {
	Title              string   `json:"title"`
	ProductRetailerIDs []string `json:"product_retailer_ids"`
}
type CloudProductsRequest struct {
	CloudInteractiveBase
	HeaderText string                `json:"header_text"`
	CatalogID  string                `json:"catalog_id"`
	Sections   []CloudProductSection `json:"sections"`
}
type CloudProductCarouselRequest struct {
	CloudInteractiveBase
	CatalogID          string   `json:"catalog_id"`
	ProductRetailerIDs []string `json:"product_retailer_ids"`
}
type CloudAddressValues struct {
	Name         string `json:"name,omitempty"`
	PhoneNumber  string `json:"phone_number,omitempty"`
	InPinCode    string `json:"in_pin_code,omitempty"`
	HouseNumber  string `json:"house_number,omitempty"`
	FloorNumber  string `json:"floor_number,omitempty"`
	TowerNumber  string `json:"tower_number,omitempty"`
	BuildingName string `json:"building_name,omitempty"`
	Address      string `json:"address,omitempty"`
	LandmarkArea string `json:"landmark_area,omitempty"`
	City         string `json:"city,omitempty"`
	State        string `json:"state,omitempty"`
}
type CloudSavedAddress struct {
	ID    string             `json:"id"`
	Value CloudAddressValues `json:"value"`
}
type CloudAddressRequest struct {
	CloudInteractiveBase
	Country          string              `json:"country"`
	Values           *CloudAddressValues `json:"values,omitempty"`
	ValidationErrors *CloudAddressValues `json:"validation_errors,omitempty"`
	SavedAddresses   []CloudSavedAddress `json:"saved_addresses,omitempty"`
}

type CloudBRAmount struct {
	Value  int64 `json:"value"`
	Offset int   `json:"offset"`
}
type CloudBRDescribedAmount struct {
	CloudBRAmount
	Description string `json:"description,omitempty"`
}
type CloudBRDiscount struct {
	CloudBRDescribedAmount
	DiscountProgramName string `json:"discount_program_name,omitempty"`
}
type CloudBRItem struct {
	RetailerID string         `json:"retailer_id"`
	Name       string         `json:"name"`
	Amount     CloudBRAmount  `json:"amount"`
	Quantity   int64          `json:"quantity"`
	SaleAmount *CloudBRAmount `json:"sale_amount,omitempty"`
}
type CloudBRExpiration struct {
	Timestamp   string `json:"timestamp"`
	Description string `json:"description"`
}
type CloudBROrder struct {
	Status     string                  `json:"status,omitempty"`
	CatalogID  string                  `json:"catalog_id,omitempty"`
	Expiration *CloudBRExpiration      `json:"expiration,omitempty"`
	Items      []CloudBRItem           `json:"items"`
	Subtotal   CloudBRAmount           `json:"subtotal"`
	Tax        CloudBRDescribedAmount  `json:"tax"`
	Shipping   *CloudBRDescribedAmount `json:"shipping,omitempty"`
	Discount   *CloudBRDiscount        `json:"discount,omitempty"`
}
type CloudBRPix struct {
	Code         string `json:"code"`
	MerchantName string `json:"merchant_name"`
	Key          string `json:"key"`
	KeyType      string `json:"key_type"`
}
type CloudBRPaymentLink struct {
	URI string `json:"uri"`
}
type CloudBRBoleto struct {
	DigitableLine string `json:"digitable_line"`
}
type CloudBRCard struct {
	LastFourDigits string `json:"last_four_digits"`
	CredentialID   string `json:"credential_id"`
}
type CloudBRPaymentSetting struct {
	Type           string              `json:"type"`
	PixDynamicCode *CloudBRPix         `json:"pix_dynamic_code,omitempty"`
	PaymentLink    *CloudBRPaymentLink `json:"payment_link,omitempty"`
	Boleto         *CloudBRBoleto      `json:"boleto,omitempty"`
	OffsiteCardPay *CloudBRCard        `json:"offsite_card_pay,omitempty"`
}
type CloudBROrderDetailsRequest struct {
	CloudInteractiveBase
	ReferenceID     string                  `json:"reference_id"`
	GoodsType       string                  `json:"goods_type"`
	TotalAmount     CloudBRAmount           `json:"total_amount"`
	PaymentSettings []CloudBRPaymentSetting `json:"payment_settings,omitempty"`
	Order           *CloudBROrder           `json:"order,omitempty"`
}
type CloudBROrderStatus struct {
	Status      string `json:"status"`
	Description string `json:"description,omitempty"`
}
type CloudBRPaymentStatus struct {
	Status    string `json:"status"`
	Timestamp *int64 `json:"timestamp,omitempty"`
}
type CloudBROrderStatusRequest struct {
	CloudInteractiveBase
	ReferenceID string                `json:"reference_id"`
	Order       *CloudBROrderStatus   `json:"order,omitempty"`
	Payment     *CloudBRPaymentStatus `json:"payment,omitempty"`
}

func cloudCatalogID(id string) error { return cloudFieldText("catalog_id", id, 128, true) }
func cloudProductID(id string) error { return cloudFieldText("product_retailer_id", id, 256, true) }
func cloudCatalogPayload(req CloudCatalogRequest) (map[string]any, error) {
	action := map[string]any{"name": "catalog_message"}
	if req.ThumbnailProductRetailerID != "" {
		if err := cloudProductID(req.ThumbnailProductRetailerID); err != nil {
			return nil, err
		}
		action["parameters"] = map[string]any{"thumbnail_product_retailer_id": req.ThumbnailProductRetailerID}
	}
	return cloudInteractivePayload(req.CloudInteractiveBase, "catalog_message", 1024, action)
}
func cloudProductPayload(req CloudProductRequest) (map[string]any, error) {
	if err := cloudCatalogID(req.CatalogID); err != nil {
		return nil, err
	}
	if err := cloudProductID(req.ProductRetailerID); err != nil {
		return nil, err
	}
	return cloudInteractivePayload(req.CloudInteractiveBase, "product", 1024, map[string]any{"catalog_id": req.CatalogID, "product_retailer_id": req.ProductRetailerID})
}
func cloudProductsPayload(req CloudProductsRequest) (map[string]any, error) {
	if err := cloudCatalogID(req.CatalogID); err != nil {
		return nil, err
	}
	sections := []map[string]any{}
	count := 0
	for _, section := range req.Sections {
		if err := cloudFieldText("section.title", section.Title, 24, true); err != nil {
			return nil, err
		}
		items := []map[string]any{}
		for _, id := range section.ProductRetailerIDs {
			if err := cloudProductID(id); err != nil {
				return nil, err
			}
			count++
			items = append(items, map[string]any{"product_retailer_id": id})
		}
		sections = append(sections, map[string]any{"title": section.Title, "product_items": items})
	}
	if count > 30 {
		return nil, cloudError("invalid_request", "multi-product messages allow at most 30 products in total")
	}
	req.Header = &CloudInteractiveHeader{Type: "text", Text: req.HeaderText}
	return cloudInteractivePayload(req.CloudInteractiveBase, "product_list", 1024, map[string]any{"catalog_id": req.CatalogID, "sections": sections})
}
func cloudProductCarouselPayload(req CloudProductCarouselRequest) (map[string]any, error) {
	if err := cloudCatalogID(req.CatalogID); err != nil {
		return nil, err
	}
	cards := []map[string]any{}
	for n, id := range req.ProductRetailerIDs {
		if err := cloudProductID(id); err != nil {
			return nil, err
		}
		cards = append(cards, map[string]any{"card_index": n, "type": "product", "action": map[string]any{"catalog_id": req.CatalogID, "product_retailer_id": id}})
	}
	return cloudInteractivePayload(req.CloudInteractiveBase, "carousel", 1024, map[string]any{"cards": cards})
}
func cloudAddressValidate(v *CloudAddressValues, validationErrors bool) error {
	if v == nil {
		return nil
	}
	raw, _ := json.Marshal(v)
	if len(raw) > 8192 {
		return cloudError("invalid_request", "address data exceeds 8192 JSON bytes (worker limit)")
	}
	if validationErrors {
		return nil
	}
	if v.InPinCode != "" && (len(v.InPinCode) > 6 || strings.IndexFunc(v.InPinCode, func(r rune) bool { return r < '0' || r > '9' }) >= 0) {
		return cloudError("invalid_request", "in_pin_code must contain at most six digits")
	}
	if v.PhoneNumber != "" {
		id, err := normalizeCloudChatID(v.PhoneNumber)
		if err != nil || !strings.HasPrefix(id, "+91") {
			return cloudError("invalid_request", "address phone_number must be an Indian international phone number")
		}
	}
	return nil
}
func cloudAddressPayload(req CloudAddressRequest) (map[string]any, error) {
	id, err := normalizeCloudChatID(req.ChatID)
	if err != nil || !strings.HasPrefix(id, "+91") {
		return nil, cloudError("invalid_request", "address requests are available only for India businesses and Indian phone recipients")
	}
	if err := cloudAddressValidate(req.Values, false); err != nil {
		return nil, err
	}
	if err := cloudAddressValidate(req.ValidationErrors, true); err != nil {
		return nil, err
	}
	ids := map[string]bool{}
	for _, saved := range req.SavedAddresses {
		if err := cloudFieldText("saved_address.id", saved.ID, 256, true); err != nil {
			return nil, err
		}
		if ids[saved.ID] {
			return nil, cloudError("invalid_request", "saved address IDs must be unique")
		}
		ids[saved.ID] = true
		if err := cloudAddressValidate(&saved.Value, false); err != nil {
			return nil, err
		}
	}
	params := map[string]any{"country": "IN"}
	if req.Values != nil {
		params["values"] = req.Values
	}
	if req.ValidationErrors != nil {
		params["validation_errors"] = req.ValidationErrors
	}
	if len(req.SavedAddresses) > 0 {
		params["saved_addresses"] = req.SavedAddresses
	}
	return cloudInteractivePayload(req.CloudInteractiveBase, "address_message", 1024, map[string]any{"name": "address_message", "parameters": params})
}

const cloudBRMaxAmount int64 = 9007199254740991

func cloudBRPositive(amount CloudBRAmount) error {
	if amount.Value == 0 {
		return cloudError("invalid_request", "totals and item prices must be positive")
	}
	return nil
}
func cloudBRDescribed(amount CloudBRDescribedAmount) error {
	return cloudFieldText("amount.description", amount.Description, 60, false)
}
func cloudBRPaymentSettings(settings []CloudBRPaymentSetting) error {
	seen := map[string]bool{}
	for _, setting := range settings {
		if seen[setting.Type] {
			return cloudError("invalid_request", "payment setting types must be unique")
		}
		seen[setting.Type] = true
		count := 0
		for _, present := range []bool{setting.PixDynamicCode != nil, setting.PaymentLink != nil, setting.Boleto != nil, setting.OffsiteCardPay != nil} {
			if present {
				count++
			}
		}
		if count != 1 {
			return cloudError("invalid_request", "each payment setting requires exactly its matching typed payload")
		}
		switch setting.Type {
		case "pix_dynamic_code":
			p := setting.PixDynamicCode
			if p == nil {
				return cloudError("invalid_request", "pix_dynamic_code payload is required")
			}
			if err := cloudFieldText("pix.code", p.Code, 4096, true); err != nil {
				return err
			}
			if err := cloudFieldText("pix.merchant_name", p.MerchantName, 255, true); err != nil {
				return err
			}
			if err := cloudFieldText("pix.key", p.Key, 255, true); err != nil {
				return err
			}
		case "payment_link":
			if setting.PaymentLink == nil {
				return cloudError("invalid_request", "payment_link payload is required")
			}
			if err := cloudDisplayURL(setting.PaymentLink.URI); err != nil {
				return err
			}
		case "boleto":
			if setting.Boleto == nil {
				return cloudError("invalid_request", "boleto payload is required")
			}
			if err := cloudFieldText("boleto.digitable_line", setting.Boleto.DigitableLine, 128, true); err != nil {
				return err
			}
		case "offsite_card_pay":
			if setting.OffsiteCardPay == nil {
				return cloudError("invalid_request", "offsite_card_pay payload is required")
			}
			p := setting.OffsiteCardPay
			if len(p.LastFourDigits) != 4 || strings.IndexFunc(p.LastFourDigits, func(r rune) bool { return r < '0' || r > '9' }) >= 0 {
				return cloudError("invalid_request", "last_four_digits must contain exactly four digits; never provide a full card number")
			}
			if err := cloudFieldText("credential_id", p.CredentialID, 256, true); err != nil {
				return err
			}
		}
	}
	return nil
}
func cloudBROrderValidate(order *CloudBROrder, total CloudBRAmount) error {
	if order == nil {
		return nil
	}
	order.Status = "pending"
	if err := cloudBRPositive(order.Subtotal); err != nil {
		return err
	}
	if err := cloudBRDescribed(order.Tax); err != nil {
		return err
	}
	if order.CatalogID != "" {
		if err := cloudCatalogID(order.CatalogID); err != nil {
			return err
		}
	}
	var sum int64
	for _, item := range order.Items {
		if err := cloudProductID(item.RetailerID); err != nil {
			return err
		}
		if err := cloudFieldText("item.name", item.Name, 60, true); err != nil {
			return err
		}
		if err := cloudBRPositive(item.Amount); err != nil {
			return err
		}
		amount := item.Amount.Value
		if item.SaleAmount != nil {
			if item.SaleAmount.Value >= amount {
				return cloudError("invalid_request", "sale_amount must be less than the original item amount")
			}
			amount = item.SaleAmount.Value
		}
		if item.Quantity < 1 || item.Quantity > cloudBRMaxAmount || amount > 0 && item.Quantity > (cloudBRMaxAmount-sum)/amount {
			return cloudError("invalid_request", "quantity must be positive and item sums must fit the JSON safe integer range")
		}
		sum += amount * item.Quantity
	}
	if sum != order.Subtotal.Value {
		return cloudError("invalid_request", "order.subtotal must equal the sum of item quantity times effective item amount")
	}
	if order.Tax.Value > cloudBRMaxAmount-sum {
		return cloudError("invalid_request", "order total exceeds the JSON safe integer range")
	}
	sum += order.Tax.Value
	if order.Shipping != nil {
		if err := cloudBRDescribed(*order.Shipping); err != nil {
			return err
		}
		if order.Shipping.Value > cloudBRMaxAmount-sum {
			return cloudError("invalid_request", "order total exceeds the JSON safe integer range")
		}
		sum += order.Shipping.Value
	}
	if order.Discount != nil {
		if err := cloudBRDescribed(order.Discount.CloudBRDescribedAmount); err != nil {
			return err
		}
		if err := cloudFieldText("discount_program_name", order.Discount.DiscountProgramName, 60, false); err != nil {
			return err
		}
		sum -= order.Discount.Value
	}
	if sum != total.Value {
		return cloudError("invalid_request", "total_amount must equal subtotal plus tax and shipping minus discount")
	}
	if order.Expiration != nil {
		timestamp, err := strconv.ParseInt(order.Expiration.Timestamp, 10, 64)
		if err != nil || timestamp < time.Now().Unix()+300 {
			return cloudError("invalid_request", "order expiration must be an epoch-seconds string at least 300 seconds in the future")
		}
		if err := cloudFieldText("expiration.description", order.Expiration.Description, 120, true); err != nil {
			return err
		}
	}
	return nil
}
func cloudBROrderDetailsPayload(req CloudBROrderDetailsRequest) (map[string]any, error) {
	if err := cloudBRPositive(req.TotalAmount); err != nil {
		return nil, err
	}
	if err := cloudBRPaymentSettings(req.PaymentSettings); err != nil {
		return nil, err
	}
	if err := cloudBROrderValidate(req.Order, req.TotalAmount); err != nil {
		return nil, err
	}
	params := map[string]any{"reference_id": req.ReferenceID, "type": req.GoodsType, "payment_type": "br", "currency": "BRL", "total_amount": req.TotalAmount}
	if len(req.PaymentSettings) > 0 {
		params["payment_settings"] = req.PaymentSettings
	}
	if req.Order != nil {
		params["order"] = req.Order
	}
	raw, _ := json.Marshal(params)
	if len(raw) > 65536 {
		return nil, cloudError("invalid_request", "order details exceed 65536 JSON bytes (worker limit)")
	}
	return cloudInteractivePayload(req.CloudInteractiveBase, "order_details", 1024, map[string]any{"name": "review_and_pay", "parameters": params})
}
func cloudBROrderStatusPayload(req CloudBROrderStatusRequest) (map[string]any, error) {
	if req.Order == nil && req.Payment == nil {
		return nil, cloudError("invalid_request", "at least one order or payment status is required")
	}
	if req.Order != nil {
		if err := cloudFieldText("order.description", req.Order.Description, 120, false); err != nil {
			return nil, err
		}
	}
	params := map[string]any{"reference_id": req.ReferenceID}
	if req.Order != nil {
		params["order"] = req.Order
	}
	if req.Payment != nil {
		params["payment"] = req.Payment
	}
	return cloudInteractivePayload(req.CloudInteractiveBase, "order_status", 1024, map[string]any{"name": "review_order", "parameters": params})
}

func registerCloudCommerce(r *Registry, client *cloudClient) {
	addCloudInteractive(r, client, "send_catalog", cloudCatalogPayload)
	addCloudInteractive(r, client, "send_product", cloudProductPayload)
	addCloudInteractive(r, client, "send_products", cloudProductsPayload)
	addCloudInteractive(r, client, "send_product_carousel", cloudProductCarouselPayload)
	addCloudInteractive(r, client, "request_address", cloudAddressPayload)
	addCloudInteractive(r, client, "send_order_details", cloudBROrderDetailsPayload)
	addCloudInteractive(r, client, "send_order_status", cloudBROrderStatusPayload)
}
