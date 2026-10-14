package main

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"io"
	"net/http"
	"strings"
	"testing"
)

func TestCloudFlowManagementWireContracts(t *testing.T) {
	for _, operation := range []string{"create", "list", "get", "update", "delete", "publish", "deprecate", "assets::list", "assets::upload", "encryption::get"} {
		t.Run(operation, func(t *testing.T) {
			calls := 0
			client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
				calls++
				if r.URL.Path == "/v26.0/999" && r.Method == "GET" {
					_, _ = w.Write([]byte(`{"id":"999","name":"Report","status":"DRAFT","whatsapp_business_account":{"id":"123"}}`))
					return
				}
				switch operation {
				case "create":
					if r.URL.Path != "/v26.0/123/flows" || r.Method != "POST" {
						t.Error("wrong create endpoint")
					}
					var body map[string]any
					_ = json.NewDecoder(r.Body).Decode(&body)
					if body["name"] != "Report" {
						t.Error("name missing")
					}
					_, _ = w.Write([]byte(`{"id":"999","success":true}`))
				case "list":
					if r.URL.Path != "/v26.0/123/flows" {
						t.Error("wrong list endpoint")
					}
					_, _ = w.Write([]byte(`{"data":[{"id":"999","name":"Report"}]}`))
				case "assets::upload":
					if r.Method != "POST" || r.URL.Path != "/v26.0/999/assets" {
						t.Error("wrong asset endpoint")
					}
					if r.ContentLength < 1 {
						t.Error("asset upload must carry Content-Length")
					}
					if err := r.ParseMultipartForm(1024 * 1024); err != nil {
						t.Fatal(err)
					}
					if r.FormValue("name") != "flow.json" || r.FormValue("asset_type") != "FLOW_JSON" {
						t.Error("wrong multipart fields")
					}
					f, _, err := r.FormFile("file")
					if err != nil {
						t.Fatal(err)
					}
					defer f.Close()
					b, _ := io.ReadAll(f)
					if string(b) != `{"version":"7.3","screens":[]}` {
						t.Error("asset changed")
					}
					_, _ = w.Write([]byte(`{"success":true,"validation_errors":[]}`))
				case "assets::list":
					_, _ = w.Write([]byte(`{"data":[{"name":"flow.json","asset_type":"FLOW_JSON","download_url":"https://scontent.xx.fbcdn.net/asset"}]}`))
				case "encryption::get":
					if r.URL.Path != "/v26.0/456/whatsapp_business_encryption" {
						t.Error("wrong encryption scope")
					}
					_, _ = w.Write([]byte(`{"business_public_key":"public","business_public_key_signature_status":"VALID"}`))
				default:
					if operation == "get" {
						t.Fatal("unnecessary second get")
					}
					want := "/v26.0/999"
					if operation == "publish" || operation == "deprecate" {
						want += "/" + operation
					}
					if r.URL.Path != want {
						t.Error("wrong mutation endpoint")
					}
					if operation == "delete" && r.Method != "DELETE" {
						t.Error("wrong delete method")
					}
					_, _ = w.Write([]byte(`{"success":true}`))
				}
			})
			input := map[string]any{"flow_id": "999"}
			switch operation {
			case "list", "encryption::get":
				input = map[string]any{}
			case "create":
				input = map[string]any{"name": "Report", "categories": []any{"OTHER"}}
			case "update":
				input["name"] = "New name"
			case "assets::upload":
				input["data_base64"] = base64.StdEncoding.EncodeToString([]byte(`{"version":"7.3","screens":[]}`))
			}
			result, err := client.manageFlow(context.Background(), cloudTestConfig(), operation, input)
			if err != nil {
				t.Fatal(err)
			}
			raw, _ := json.Marshal(result)
			if strings.Contains(string(raw), "download_url") || strings.Contains(string(raw), "test-access-secret") {
				t.Fatal("unsafe result")
			}
			if calls == 0 {
				t.Fatal("no HTTP request")
			}
		})
	}
}

func TestCloudFlowRejectsForeignAccountAndInvalidAssets(t *testing.T) {
	mutations := 0
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		if r.Method != "GET" {
			mutations++
		}
		_, _ = w.Write([]byte(`{"id":"999","whatsapp_business_account":{"id":"other"}}`))
	})
	_, err := client.manageFlow(context.Background(), cloudTestConfig(), "publish", map[string]any{"flow_id": "999"})
	assertCloudDocumentError(t, err, "cloud_resource_forbidden")
	if mutations != 0 {
		t.Fatal("foreign Flow mutated")
	}
	_, err = client.manageFlow(context.Background(), cloudTestConfig(), "assets::upload", map[string]any{"flow_id": "999", "data_base64": base64.StdEncoding.EncodeToString([]byte(`not-json`))})
	assertCloudDocumentError(t, err, "invalid_request")
	registry := &Registry{handlers: map[string]operationHandler{}}
	registerCloudFlows(registry, client)
	_, err = registry.Invoke(context.Background(), "whatsapp::cloud::flows::create", map[string]any{"name": "Report", "categories": []any{"MADE_UP"}})
	assertCloudDocumentError(t, err, "invalid_request")
}

func TestCloudFlowAssetDownloadOriginBytesAndBound(t *testing.T) {
	for _, bad := range []string{"", "host", "mime", "size", "redirect"} {
		t.Run(bad, func(t *testing.T) {
			client := newCloudClient()
			calls := 0
			data := []byte(`{"version":"7.3","screens":[]}`)
			client.http.Transport = cloudMediaTestTransport(func(r *http.Request) (*http.Response, error) {
				calls++
				if r.URL.Host != "scontent.xx.fbcdn.net" || r.Header.Get("Authorization") != "Bearer test-access-secret" {
					t.Fatal("unexpected download")
				}
				mime := "application/json"
				if bad == "mime" {
					mime = "text/html"
				}
				status := 200
				if bad == "redirect" {
					status = 302
				}
				response := cloudMediaHTTPResponse(status, mime, data)
				if bad == "mime" {
					response.ContentLength = -1 // an oversized body of the wrong type is rejected before it is read
				}
				return response, nil
			})
			uri := "https://scontent.xx.fbcdn.net/asset"
			if bad == "host" {
				uri = "https://attacker.example/asset"
			}
			response := map[string]any{"data": []any{map[string]any{"name": "flow.json", "asset_type": "FLOW_JSON", "download_url": uri}}}
			config := cloudTestConfig()
			if bad == "size" || bad == "mime" {
				config.MaxMediaBytes = 10
			}
			result, err := client.downloadFlowAsset(context.Background(), config, response)
			if bad == "mime" {
				assertCloudDocumentError(t, err, "cloud_unsupported_media")
			}
			if bad == "" {
				if err != nil {
					t.Fatal(err)
				}
				if result.(map[string]any)["data_base64"] != base64.StdEncoding.EncodeToString(data) {
					t.Fatal("bytes changed")
				}
			} else if err == nil {
				t.Fatal("unsafe download accepted")
			}
			if bad == "host" && calls != 0 {
				t.Fatal("untrusted request dispatched")
			}
			if calls > 1 {
				t.Fatal("download retried")
			}
		})
	}
}

func TestCloudFlowValidationFailuresRemainInspectable(t *testing.T) {
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		if r.Method == "GET" {
			_, _ = w.Write([]byte(`{"id":"999","whatsapp_business_account":{"id":"123"}}`))
			return
		}
		_, _ = w.Write([]byte(`{"success":false,"validation_errors":[{"error":"INVALID_PROPERTY_VALUE","message":"Invalid test-access-secret"}]}`))
	})
	result, err := client.manageFlow(context.Background(), cloudTestConfig(), "assets::upload", map[string]any{"flow_id": "999", "data_base64": base64.StdEncoding.EncodeToString([]byte(`{"screens":[]}`))})
	if err != nil {
		t.Fatal(err)
	}
	raw, _ := json.Marshal(result)
	if !strings.Contains(string(raw), "INVALID_PROPERTY_VALUE") || strings.Contains(string(raw), "test-access-secret") {
		t.Fatal("unsafe or missing validation result")
	}
}
