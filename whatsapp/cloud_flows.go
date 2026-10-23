package main

import (
	"bytes"
	"context"
	"crypto/rsa"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"io"
	"net/http"
	"net/url"
	"strings"
)

type CloudFlowMutationRequest struct {
	FlowID      string         `json:"flow_id,omitempty"`
	Name        string         `json:"name,omitempty"`
	Categories  []string       `json:"categories,omitempty"`
	CloneFlowID string         `json:"clone_flow_id,omitempty"`
	EndpointURI string         `json:"endpoint_uri,omitempty"`
	FlowJSON    map[string]any `json:"flow_json,omitempty"`
	Publish     bool           `json:"publish,omitempty"`
}
type CloudFlowIDRequest struct {
	FlowID string `json:"flow_id"`
}
type CloudFlowAssetRequest struct {
	FlowID     string `json:"flow_id"`
	DataBase64 string `json:"data_base64"`
}
type CloudFlowEncryptionRequest struct {
	PublicKey string `json:"public_key,omitempty"`
}

var cloudFlowOperations = []string{"create", "list", "get", "update", "delete", "publish", "deprecate", "preview", "assets::list", "assets::upload", "assets::download", "encryption::get", "encryption::set", "endpoint::status"}

const cloudFlowFields = "id,name,status,categories,validation_errors,json_version,data_api_version,endpoint_uri,whatsapp_business_account"

func cloudFlowInvalid() error {
	return cloudError("invalid_request", "Invalid Flow request; check the function schema and supported Flow categories")
}

func (c *cloudClient) ownedFlow(ctx context.Context, config Config, id string) (map[string]any, error) {
	if !cloudDocumentMediaID(id) {
		return nil, cloudFlowInvalid()
	}
	response, err := c.request(ctx, config, http.MethodGet, id, url.Values{"fields": {cloudFlowFields}}, nil)
	if err != nil {
		return nil, err
	}
	account := cloudNested(response, "whatsapp_business_account")
	if account["id"] != config.Cloud.BusinessAccountID {
		return nil, cloudError("cloud_resource_forbidden", "Flow does not belong to the configured WhatsApp Business Account")
	}
	return response, nil
}

func cloudFlowSafePage(config Config, response map[string]any, assets bool) (map[string]any, error) {
	items, err := cloudArray(response["data"])
	if err != nil {
		return nil, cloudError("cloud_invalid_response", "Cloud API returned an invalid Flow collection")
	}
	clean := make([]any, 0, len(items))
	for _, item := range items {
		object, ok := item.(map[string]any)
		if !ok {
			return nil, cloudError("cloud_invalid_response", "Invalid Flow collection item")
		}
		if assets {
			object = map[string]any{"name": object["name"], "asset_type": object["asset_type"]}
		}
		clean = append(clean, cloudSanitizeResult(config, object))
	}
	cursor, err := cloudNextCursor(response)
	if err != nil {
		return nil, cloudError("cloud_invalid_response", "Invalid Flow pagination cursor")
	}
	return cloudSanitizeResult(config, map[string]any{"data": clean, "next_cursor": cursor}).(map[string]any), nil
}

func (c *cloudClient) manageFlow(ctx context.Context, config Config, operation string, input map[string]any) (any, error) {
	if operation == "endpoint::status" {
		return map[string]any{"path": cloudFlowPath(), "configured": config.Cloud.Enabled && config.Cloud.FlowPrivateKey != "" && config.Cloud.AppSecret != "" && config.Cloud.FlowHandlerFunction != "", "private_key_set": config.Cloud.FlowPrivateKey != "", "handler_function": config.Cloud.FlowHandlerFunction, "handler_namespace": config.Cloud.FlowHandlerNamespace}, nil
	}
	if err := cloudRequestConfig(config); err != nil {
		return nil, err
	}
	var response map[string]any
	var err error
	switch operation {
	case "create", "update":
		var req CloudFlowMutationRequest
		if decodeCloudRequest(input, &req) != nil {
			return nil, cloudFlowInvalid()
		}
		if !cloudManagementText(req.Name, 1, 256) {
			return nil, cloudFlowInvalid()
		}
		if req.EndpointURI != "" && !cloudManagementURL(req.EndpointURI, true) {
			return nil, cloudFlowInvalid()
		}
		body := map[string]any{"name": req.Name}
		if req.Categories != nil {
			body["categories"] = req.Categories
		}
		if req.EndpointURI != "" {
			body["endpoint_uri"] = req.EndpointURI
		}
		path := config.Cloud.BusinessAccountID + "/flows"
		if operation == "create" {
			if req.CloneFlowID != "" {
				if _, err = c.ownedFlow(ctx, config, req.CloneFlowID); err != nil {
					return nil, err
				}
				body["clone_flow_id"] = req.CloneFlowID
			}
			if req.FlowJSON != nil {
				raw, e := json.Marshal(req.FlowJSON)
				if e != nil || len(raw) > min(config.MaxMediaBytes, 10_000_000) {
					return nil, cloudFlowInvalid()
				}
				body["flow_json"] = string(raw)
			}
			if req.Publish {
				body["publish"] = true
			}
		} else {
			if _, err = c.ownedFlow(ctx, config, req.FlowID); err != nil {
				return nil, err
			}
			path = req.FlowID
		}
		response, err = c.request(ctx, config, http.MethodPost, path, nil, body)
	case "list":
		query, e := cloudPageQuery(input, 50)
		if e != nil {
			return nil, cloudFlowInvalid()
		}
		query.Set("fields", cloudFlowFields)
		response, err = c.request(ctx, config, http.MethodGet, config.Cloud.BusinessAccountID+"/flows", query, nil)
		if err == nil {
			return cloudFlowSafePage(config, response, false)
		}
	case "assets::upload":
		var req CloudFlowAssetRequest
		if decodeCloudRequest(input, &req) != nil {
			return nil, cloudFlowInvalid()
		}
		limit := min(config.MaxMediaBytes, 10_000_000)
		if limit < 1 || len(req.DataBase64) > base64.StdEncoding.EncodedLen(limit) {
			return nil, cloudError("cloud_media_too_large", "Flow JSON exceeds the configured limit or 10 MB")
		}
		raw, e := base64.StdEncoding.Strict().DecodeString(req.DataBase64)
		var document map[string]any
		if e != nil || len(raw) == 0 || len(raw) > limit || json.Unmarshal(raw, &document) != nil || document == nil {
			return nil, cloudFlowInvalid()
		}
		if _, err = c.ownedFlow(ctx, config, req.FlowID); err != nil {
			return nil, err
		}
		body, contentType := cloudMultipartFile("flow.json", "application/json", raw, "name", "flow.json", "asset_type", "FLOW_JSON")
		envelope, _ := io.ReadAll(body) // buffered so the request keeps its Content-Length
		response, err = c.requestBody(ctx, config, http.MethodPost, req.FlowID+"/assets", nil, bytes.NewReader(envelope), contentType)
	case "encryption::get", "encryption::set":
		var req CloudFlowEncryptionRequest
		if decodeCloudRequest(input, &req) != nil {
			return nil, cloudFlowInvalid()
		}
		if operation == "encryption::get" {
			response, err = c.request(ctx, config, http.MethodGet, config.Cloud.PhoneNumberID+"/whatsapp_business_encryption", nil, nil)
		} else {
			if req.PublicKey == "" {
				key, e := cloudFlowPrivateKey(config.Cloud.FlowPrivateKey)
				if e != nil {
					return nil, cloudError("cloud_not_configured", "Configure a Flow private key or provide its RSA-2048 public key")
				}
				der, _ := x509.MarshalPKIXPublicKey(&key.PublicKey)
				req.PublicKey = string(pem.EncodeToMemory(&pem.Block{Type: "PUBLIC KEY", Bytes: der}))
			}
			block, rest := pem.Decode([]byte(req.PublicKey))
			if len(req.PublicKey) > 16384 || block == nil || block.Type != "PUBLIC KEY" || strings.TrimSpace(string(rest)) != "" {
				return nil, cloudFlowInvalid()
			}
			key, e := x509.ParsePKIXPublicKey(block.Bytes)
			rsaKey, ok := key.(*rsa.PublicKey)
			if e != nil || !ok || rsaKey.N.BitLen() != 2048 {
				return nil, cloudFlowInvalid()
			}
			body := url.Values{"business_public_key": {req.PublicKey}}
			response, err = c.requestBody(ctx, config, http.MethodPost, config.Cloud.PhoneNumberID+"/whatsapp_business_encryption", nil, strings.NewReader(body.Encode()), "application/x-www-form-urlencoded")
		}
	default:
		var req CloudFlowIDRequest
		if decodeCloudRequest(input, &req) != nil {
			return nil, cloudFlowInvalid()
		}
		owned, e := c.ownedFlow(ctx, config, req.FlowID)
		if e != nil {
			return nil, e
		}
		switch operation {
		case "get":
			return cloudSanitizeResult(config, owned), nil
		case "delete":
			response, err = c.request(ctx, config, http.MethodDelete, req.FlowID, nil, nil)
		case "publish", "deprecate":
			response, err = c.request(ctx, config, http.MethodPost, req.FlowID+"/"+operation, nil, map[string]any{})
		case "preview":
			response, err = c.request(ctx, config, http.MethodGet, req.FlowID, url.Values{"fields": {"preview.invalidate(false)"}}, nil)
		case "assets::list", "assets::download":
			response, err = c.request(ctx, config, http.MethodGet, req.FlowID+"/assets", nil, nil)
			if err == nil && operation == "assets::list" {
				return cloudFlowSafePage(config, response, true)
			}
			if err == nil {
				return c.downloadFlowAsset(ctx, config, response)
			}
		}
	}
	if err != nil {
		return nil, err
	}
	if operation == "create" {
		if id, ok := response["id"].(string); !ok || !cloudDocumentMediaID(id) {
			return nil, cloudResponseError(http.MethodPost, "Flow creation returned no valid ID")
		}
	} else if operation == "update" || operation == "delete" || operation == "publish" || operation == "deprecate" || operation == "encryption::set" || operation == "assets::upload" {
		if _, ok := response["success"].(bool); !ok {
			return nil, cloudResponseError(http.MethodPost, "Meta did not confirm the Flow operation; inspect its state before another request")
		}
	}
	return cloudSanitizeResult(config, response), nil
}

func (c *cloudClient) downloadFlowAsset(ctx context.Context, config Config, response map[string]any) (any, error) {
	items, err := cloudArray(response["data"])
	if err != nil {
		return nil, cloudError("cloud_invalid_response", "Invalid Flow asset collection")
	}
	rawURL := ""
	for _, item := range items {
		object, _ := item.(map[string]any)
		if object["name"] == "flow.json" && object["asset_type"] == "FLOW_JSON" {
			rawURL, _ = object["download_url"].(string)
			break
		}
	}
	u, err := cloudMediaDownloadURL(rawURL)
	if err != nil {
		return nil, err
	}
	raw, err := c.guardedDownload(ctx, config, u, min(config.MaxMediaBytes, 10_000_000), []string{"application/json", "text/plain"}, cloudError("cloud_download_error", "Unable to read Flow asset"))
	if err != nil {
		return nil, err
	}
	var document map[string]any
	if json.Unmarshal(raw, &document) != nil || document == nil {
		return nil, cloudError("cloud_invalid_response", "Downloaded Flow asset is not a JSON object")
	}
	return map[string]any{"name": "flow.json", "asset_type": "FLOW_JSON", "mimetype": "application/json", "file_length": len(raw), "data_base64": base64.StdEncoding.EncodeToString(raw)}, nil
}

func registerCloudFlows(r *Registry, client *cloudClient) {
	for _, operation := range cloudFlowOperations {
		r.Add("whatsapp::cloud::flows::"+operation, func(ctx context.Context, input map[string]any) (any, error) {
			config, configErr := r.Manager.ConfigContext(ctx)
			if configErr != nil {
				return nil, configErr
			}
			return client.manageFlow(ctx, config, operation, input)
		})
	}
}
