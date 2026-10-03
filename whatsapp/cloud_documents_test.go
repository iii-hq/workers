package main

import (
	"archive/zip"
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"mime"
	"net/http"
	"net/http/httptest"
	"net/url"
	"reflect"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

func assertCloudDocumentError(t *testing.T, err error, code string) {
	t.Helper()
	var remote *iii.InvocationError
	if !errors.As(err, &remote) || remote.Code != code {
		t.Fatalf("expected structured %s error, got %v", code, err)
	}
	serialized, marshalErr := json.Marshal(remote)
	if marshalErr != nil {
		t.Fatal(marshalErr)
	}
	for _, secret := range []string{cloudTestConfig().Cloud.AccessToken, cloudTestConfig().Cloud.AppSecret, cloudTestConfig().Cloud.VerifyToken} {
		for _, encoded := range []string{secret, url.QueryEscape(secret), base64.StdEncoding.EncodeToString([]byte(secret))} {
			if strings.Contains(err.Error()+string(serialized), encoded) {
				t.Fatal("error exposed a credential")
			}
		}
	}
}

func documentUploadRequest(data []byte) CloudDocumentUploadRequest {
	return CloudDocumentUploadRequest{DataBase64: base64.StdEncoding.EncodeToString(data), Mimetype: "text/plain", FileName: "report.txt"}
}

func documentSendRequest() CloudDocumentRequest {
	return CloudDocumentRequest{ChatID: "+15551234567", MediaID: "987654321", FileName: "report.txt"}
}

func TestCloudDocumentMultipartAndReusableMediaID(t *testing.T) {
	data := []byte("Report for café\nFirst line\r\nSecond line\n")
	fileName := "Relatório \"October\".txt"
	var uploads, sends atomic.Int32
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPost || r.Header.Get("Authorization") != "Bearer test-access-secret" {
			t.Error("unexpected method or missing authorization")
		}
		if strings.Contains(r.URL.RawQuery, "access_token") {
			t.Error("credential placed in URL")
		}
		switch r.URL.Path {
		case "/v26.0/456/media":
			uploads.Add(1)
			reader, err := r.MultipartReader()
			if err != nil {
				t.Error(err)
				w.WriteHeader(http.StatusBadRequest)
				return
			}
			fields := map[string]string{}
			for {
				part, err := reader.NextPart()
				if err == io.EOF {
					break
				}
				if err != nil {
					t.Error(err)
					return
				}
				content, err := io.ReadAll(part)
				if err != nil {
					t.Error(err)
					return
				}
				if _, exists := fields[part.FormName()]; exists {
					t.Errorf("duplicate multipart field %q", part.FormName())
				}
				fields[part.FormName()] = string(content)
				if part.FormName() == "file" {
					if strings.Contains(part.Header.Get("Content-Disposition"), "filename*=") || !strings.Contains(part.Header.Get("Content-Disposition"), `filename="Relatório \"October\".txt"`) {
						t.Error("multipart filename must use quoted UTF-8, without forbidden filename* encoding")
					}
					_, disposition, err := mime.ParseMediaType(part.Header.Get("Content-Disposition"))
					if err != nil || disposition["filename"] != fileName || part.Header.Get("Content-Type") != "text/plain" {
						t.Errorf("incorrect file part: filename=%q content-type=%q err=%v", disposition["filename"], part.Header.Get("Content-Type"), err)
					}
				}
			}
			want := map[string]string{"messaging_product": "whatsapp", "type": "text/plain", "file": string(data)}
			if !reflect.DeepEqual(fields, want) {
				t.Errorf("multipart bytes or fields changed: %#v", fields)
			}
			_, _ = io.WriteString(w, `{"id":"987654321"}`)
		case "/v26.0/456/messages":
			call := sends.Add(1)
			var body map[string]any
			if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
				t.Error(err)
			}
			want := map[string]any{
				"messaging_product": "whatsapp", "recipient_type": "individual", "to": "+15551234567", "type": "document",
				"document": map[string]any{"id": "987654321", "filename": fileName, "caption": "Monthly report"},
			}
			if !reflect.DeepEqual(body, want) {
				t.Errorf("unexpected document payload: %#v", body)
			}
			_, _ = fmt.Fprintf(w, `{"messages":[{"id":"wamid.document.%d","message_status":"accepted"}]}`, call)
		default:
			t.Errorf("unexpected Cloud API path %q", r.URL.Path)
			w.WriteHeader(http.StatusNotFound)
		}
	})
	config := cloudTestConfig()
	config.DataDir = t.TempDir()
	manager, err := NewManager(config)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = manager.Shutdown(context.Background()) })
	registry := &Registry{Manager: manager, handlers: map[string]operationHandler{}}
	registerCloudDocuments(registry, client)
	uploaded, err := registry.Invoke(context.Background(), "whatsapp::cloud::media::upload_document", map[string]any{
		"data_base64": base64.StdEncoding.EncodeToString(data), "mimetype": "text/plain", "file_name": fileName, "_caller_worker_id": "authorized-test-caller",
	})
	if err != nil {
		t.Fatal(err)
	}
	wantUpload := CloudUploadedDocument{MediaID: "987654321", Mimetype: "text/plain", FileName: fileName, FileLength: len(data)}
	if !reflect.DeepEqual(uploaded, wantUpload) {
		t.Fatalf("unexpected upload acknowledgement: %#v", uploaded)
	}
	for i := range 2 {
		sent, err := registry.Invoke(context.Background(), "whatsapp::cloud::messages::send_document", map[string]any{
			"chat_id": "+15551234567", "media_id": wantUpload.MediaID, "file_name": fileName, "caption": "Monthly report",
		})
		if err != nil {
			t.Fatal(err)
		}
		result, ok := sent.(CloudSentMessage)
		if !ok || result.MessageID != fmt.Sprintf("wamid.document.%d", i+1) || result.Status != "accepted" || result.MessageStatus == nil || *result.MessageStatus != "accepted" {
			t.Fatalf("unexpected send acknowledgement: %#v", sent)
		}
	}
	if uploads.Load() != 1 || sends.Load() != 2 {
		t.Fatalf("media was not reused: uploads=%d sends=%d", uploads.Load(), sends.Load())
	}
	inbox, err := manager.CloudInbox(config)
	if err != nil {
		t.Fatal(err)
	}
	page, err := inbox.ListMessages("cloud:+15551234567", "", 10)
	if err != nil || len(page.Messages) != 2 {
		t.Fatalf("document archive missing: %+v %v", page, err)
	}
	for _, message := range page.Messages {
		if message.Kind != "document" || message.Text != "Monthly report" || message.Status == nil || *message.Status != "submitted" {
			t.Fatalf("API acceptance was not archived as an unconfirmed document: %+v", message)
		}
	}
	change := cloudTestChange()
	delete(change["value"].(map[string]any), "messages")
	change["value"].(map[string]any)["statuses"] = []any{map[string]any{
		"id": "wamid.document.1", "status": "delivered", "timestamp": "1700000010", "recipient_id": "15551234567",
	}}
	batch := cloudTestParse(t, cloudTestPayload(change))
	if _, err := inbox.RecordExternal(batch.messages, batch.receipts); err != nil {
		t.Fatal(err)
	}
	page, err = inbox.ListMessages("cloud:+15551234567", "", 10)
	if err != nil {
		t.Fatal(err)
	}
	for _, message := range page.Messages {
		wantStatus := "submitted"
		if message.MessageID == "wamid.document.1" {
			wantStatus = "delivered"
		}
		if message.Status == nil || *message.Status != wantStatus {
			t.Fatalf("delivery status must change only for the confirmed webhook message: %+v", message)
		}
	}
	for _, id := range []string{"whatsapp::cloud::media::upload_document", "whatsapp::cloud::messages::send_document"} {
		if typedInstallers[id] == nil {
			t.Errorf("missing typed SDK registration for %s", id)
		}
	}
}

func TestCloudDocumentUploadValidationBeforeHTTP(t *testing.T) {
	var calls atomic.Int32
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		_, _ = io.WriteString(w, `{"id":"987654321"}`)
	})
	tests := []struct {
		name string
		edit func(*CloudDocumentUploadRequest, *Config)
		code string
	}{
		{"markdown", func(r *CloudDocumentUploadRequest, _ *Config) { r.Mimetype = "text/markdown"; r.FileName = "report.md" }, "cloud_unsupported_document"},
		{"markdown_claiming_plain_text", func(r *CloudDocumentUploadRequest, _ *Config) { r.FileName = "report.md" }, "cloud_unsupported_document"},
		{"unsupported_image", func(r *CloudDocumentUploadRequest, _ *Config) { r.Mimetype = "image/png"; r.FileName = "report.png" }, "cloud_unsupported_document"},
		{"mime_parameters", func(r *CloudDocumentUploadRequest, _ *Config) { r.Mimetype = "text/plain; charset=utf-8" }, "cloud_unsupported_document"},
		{"extension_mismatch", func(r *CloudDocumentUploadRequest, _ *Config) { r.FileName = "report.pdf" }, "cloud_unsupported_document"},
		{"empty_file_name", func(r *CloudDocumentUploadRequest, _ *Config) { r.FileName = "" }, "invalid_request"},
		{"absolute_path", func(r *CloudDocumentUploadRequest, _ *Config) { r.FileName = "/tmp/report.txt" }, "invalid_request"},
		{"relative_path", func(r *CloudDocumentUploadRequest, _ *Config) { r.FileName = "../report.txt" }, "invalid_request"},
		{"windows_path", func(r *CloudDocumentUploadRequest, _ *Config) { r.FileName = `C:\reports\report.txt` }, "invalid_request"},
		{"header_injection", func(r *CloudDocumentUploadRequest, _ *Config) { r.FileName = "report\r\nX-Injected: yes.txt" }, "invalid_request"},
		{"file_name_byte_limit", func(r *CloudDocumentUploadRequest, _ *Config) { r.FileName = strings.Repeat("é", 126) + ".txt" }, "invalid_request"},
		{"invalid_utf8_file_name", func(r *CloudDocumentUploadRequest, _ *Config) { r.FileName = string([]byte{0xff}) + ".txt" }, "invalid_request"},
		{"empty_data", func(r *CloudDocumentUploadRequest, _ *Config) { r.DataBase64 = "" }, "invalid_request"},
		{"invalid_base64", func(r *CloudDocumentUploadRequest, _ *Config) { r.DataBase64 = "not-base64!" }, "invalid_request"},
		{"non_utf8_text", func(r *CloudDocumentUploadRequest, _ *Config) {
			r.DataBase64 = base64.StdEncoding.EncodeToString([]byte{0xff})
		}, "cloud_unsupported_document"},
		{"binary_text", func(r *CloudDocumentUploadRequest, _ *Config) {
			r.DataBase64 = base64.StdEncoding.EncodeToString([]byte("report\x00data"))
		}, "cloud_unsupported_document"},
		{"pdf_signature_mismatch", func(r *CloudDocumentUploadRequest, _ *Config) {
			r.Mimetype = "application/pdf"
			r.FileName = "report.pdf"
		}, "cloud_unsupported_document"},
		{"exceeds_decoded_limit", func(r *CloudDocumentUploadRequest, c *Config) {
			c.MaxMediaBytes = 3
			r.DataBase64 = base64.StdEncoding.EncodeToString([]byte("four"))
		}, "cloud_media_too_large"},
		{"reject_oversized_before_decode", func(r *CloudDocumentUploadRequest, c *Config) {
			c.MaxMediaBytes = 3
			r.DataBase64 = strings.Repeat("!", 100)
		}, "cloud_media_too_large"},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			req, config := documentUploadRequest([]byte("report\n")), cloudTestConfig()
			tc.edit(&req, &config)
			_, err := client.uploadDocument(context.Background(), config, req)
			assertCloudDocumentError(t, err, tc.code)
			if strings.HasPrefix(tc.name, "markdown") && (!strings.Contains(err.Error(), ".txt") || !strings.Contains(strings.ToLower(err.Error()), "pdf")) {
				t.Error("Markdown error must offer supported alternatives requiring caller choice")
			}
		})
	}
	if calls.Load() != 0 {
		t.Fatalf("invalid documents caused %d HTTP requests", calls.Load())
	}
	config := cloudTestConfig()
	config.MaxMediaBytes = 4
	if uploaded, err := client.uploadDocument(context.Background(), config, documentUploadRequest([]byte("four"))); err != nil || uploaded.FileLength != 4 {
		t.Fatalf("document at configured byte limit failed: %#v %v", uploaded, err)
	}
}

func TestCloudDocumentPayloadValidation(t *testing.T) {
	caption := strings.Repeat("é", 1024)
	req := documentSendRequest()
	req.ChatID = "BR.ENT.test123"
	req.Caption = &caption
	body, err := cloudDocumentPayload(req)
	if err != nil {
		t.Fatal(err)
	}
	document := body["document"].(map[string]any)
	if body["recipient"] != req.ChatID || body["to"] != nil || document["caption"] != caption || document["id"] != req.MediaID || document["filename"] != req.FileName {
		t.Fatalf("invalid BSUID document payload: %#v", body)
	}
	req = documentSendRequest()
	body, err = cloudDocumentPayload(req)
	if err != nil {
		t.Fatal(err)
	}
	if _, exists := body["document"].(map[string]any)["caption"]; exists {
		t.Fatal("absent caption was sent")
	}
	for _, tc := range []struct {
		name string
		edit func(*CloudDocumentRequest)
	}{
		{"missing_recipient", func(r *CloudDocumentRequest) { r.ChatID = "" }},
		{"native_user_jid", func(r *CloudDocumentRequest) { r.ChatID = "15551234567@s.whatsapp.net" }},
		{"native_group_jid", func(r *CloudDocumentRequest) { r.ChatID = "12345@g.us" }},
		{"missing_media_id", func(r *CloudDocumentRequest) { r.MediaID = "" }},
		{"media_url", func(r *CloudDocumentRequest) { r.MediaID = "https://example.invalid/report.txt" }},
		{"media_path", func(r *CloudDocumentRequest) { r.MediaID = "../987654321" }},
		{"overlong_media_id", func(r *CloudDocumentRequest) { r.MediaID = strings.Repeat("1", 129) }},
		{"missing_filename", func(r *CloudDocumentRequest) { r.FileName = "" }},
		{"filename_path", func(r *CloudDocumentRequest) { r.FileName = "/tmp/report.txt" }},
		{"caption_character_limit", func(r *CloudDocumentRequest) { value := strings.Repeat("é", 1025); r.Caption = &value }},
		{"caption_nul", func(r *CloudDocumentRequest) { value := "a\x00b"; r.Caption = &value }},
		{"caption_invalid_utf8", func(r *CloudDocumentRequest) { value := string([]byte{0xff}); r.Caption = &value }},
	} {
		t.Run(tc.name, func(t *testing.T) {
			req := documentSendRequest()
			tc.edit(&req)
			_, err := cloudDocumentPayload(req)
			assertCloudDocumentError(t, err, "invalid_request")
		})
	}
}

func TestCloudDocumentContractsRejectUntrustedTransfers(t *testing.T) {
	registry := NewRegistry(nil)
	for _, tc := range []struct {
		id   string
		base map[string]any
	}{
		{"whatsapp::cloud::media::upload_document", map[string]any{"data_base64": "YQ==", "mimetype": "text/plain", "file_name": "a.txt"}},
		{"whatsapp::cloud::messages::send_document", map[string]any{"chat_id": "+15551234567", "media_id": "123", "file_name": "a.txt"}},
	} {
		for _, field := range []string{"path", "url", "link", "session_id", "access_token"} {
			input := make(map[string]any, len(tc.base)+1)
			for key, value := range tc.base {
				input[key] = value
			}
			input[field] = "untrusted-input"
			_, err := registry.Invoke(context.Background(), tc.id, input)
			assertCloudDocumentError(t, err, "invalid_request")
			if strings.Contains(err.Error(), "untrusted-input") {
				t.Fatal("schema error echoed private request content")
			}
		}
	}
	for _, value := range []any{nil, 42} {
		_, err := registry.Invoke(context.Background(), "whatsapp::cloud::media::upload_document", map[string]any{"data_base64": value, "mimetype": "text/plain", "file_name": "a.txt"})
		assertCloudDocumentError(t, err, "invalid_request")
	}
}

func TestCloudDocumentCredentialsRequiredBeforeHTTP(t *testing.T) {
	var calls atomic.Int32
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) { calls.Add(1); _, _ = io.WriteString(w, `{"id":"123"}`) })
	for _, tc := range []struct {
		name string
		edit func(*Config)
		code string
	}{
		{"disabled", func(c *Config) { c.Cloud.Enabled = false }, "cloud_disabled"},
		{"missing_access_token", func(c *Config) { c.Cloud.AccessToken = "" }, "cloud_not_configured"},
		{"missing_phone_number_id", func(c *Config) { c.Cloud.PhoneNumberID = "" }, "cloud_not_configured"},
		{"invalid_token_header", func(c *Config) { c.Cloud.AccessToken = "test-access-secret\r\nInjected: yes" }, "cloud_not_configured"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			config := cloudTestConfig()
			tc.edit(&config)
			_, err := client.uploadDocument(context.Background(), config, documentUploadRequest([]byte("report")))
			assertCloudDocumentError(t, err, tc.code)
			body, err := cloudDocumentPayload(documentSendRequest())
			if err != nil {
				t.Fatal(err)
			}
			_, err = client.send(context.Background(), config, "+15551234567", body)
			assertCloudDocumentError(t, err, tc.code)
		})
	}
	if calls.Load() != 0 {
		t.Fatalf("missing credentials caused %d network requests", calls.Load())
	}
}

func TestCloudDocumentRemoteErrorsNoRetriesOrCredentialLeak(t *testing.T) {
	for _, tc := range []struct {
		name             string
		status, metaCode int
		code             string
	}{
		{"expired_credential", 401, 190, "cloud_api_error"},
		{"closed_service_window", 400, 131047, "cloud_window_closed"},
		{"rate_limit", 429, 130429, "cloud_api_error"},
		{"temporary_failure", 503, 131000, "cloud_api_error"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			var calls atomic.Int32
			client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
				calls.Add(1)
				w.WriteHeader(tc.status)
				private := "remote-private-details " + cloudTestConfig().Cloud.AccessToken + " " + base64.StdEncoding.EncodeToString([]byte(cloudTestConfig().Cloud.AccessToken))
				_ = json.NewEncoder(w).Encode(map[string]any{"error": map[string]any{"code": tc.metaCode, "message": private, "error_data": map[string]any{"details": private}, "fbtrace_id": private}})
			})
			_, err := client.uploadDocument(context.Background(), cloudTestConfig(), documentUploadRequest([]byte("report")))
			assertCloudDocumentError(t, err, tc.code)
			if !strings.Contains(err.Error(), fmt.Sprint(tc.metaCode)) || strings.Contains(err.Error(), "remote-private-details") {
				t.Fatalf("unsafe or unactionable error: %v", err)
			}
			body, err := cloudDocumentPayload(documentSendRequest())
			if err != nil {
				t.Fatal(err)
			}
			_, err = client.send(context.Background(), cloudTestConfig(), "+15551234567", body)
			assertCloudDocumentError(t, err, tc.code)
			if calls.Load() != 2 {
				t.Fatalf("API rejection caused retry or fallback: %d calls", calls.Load())
			}
		})
	}
}

func TestCloudDocumentAmbiguousTimeoutDoesNotRetry(t *testing.T) {
	for _, operation := range []string{"upload", "send"} {
		t.Run(operation, func(t *testing.T) {
			var calls atomic.Int32
			release := make(chan struct{})
			client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
				_, _ = io.Copy(io.Discard, r.Body)
				calls.Add(1)
				<-release // The API accepted the bytes, but no acknowledgement arrived.
			})
			defer close(release)
			ctx, cancel := context.WithTimeout(context.Background(), 100*time.Millisecond)
			defer cancel()
			var err error
			if operation == "upload" {
				_, err = client.uploadDocument(ctx, cloudTestConfig(), documentUploadRequest([]byte("report")))
			} else {
				body, payloadErr := cloudDocumentPayload(documentSendRequest())
				if payloadErr != nil {
					t.Fatal(payloadErr)
				}
				_, err = client.send(ctx, cloudTestConfig(), "+15551234567", body)
			}
			assertCloudDocumentError(t, err, "cloud_outcome_unknown")
			if !strings.Contains(strings.ToLower(err.Error()), "may have") && !strings.Contains(strings.ToLower(err.Error()), "unknown") {
				t.Fatal("timeout did not explain ambiguous outcome")
			}
			if calls.Load() != 1 {
				t.Fatalf("ambiguous request was retried: %d calls", calls.Load())
			}
		})
	}
}

func TestCloudDocumentRedirectAndInvalidAcknowledgement(t *testing.T) {
	var escaped atomic.Int32
	target := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { escaped.Add(1) }))
	defer target.Close()
	var calls atomic.Int32
	redirect := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		w.Header().Set("Location", target.URL)
		w.WriteHeader(http.StatusTemporaryRedirect)
	})
	_, err := redirect.uploadDocument(context.Background(), cloudTestConfig(), documentUploadRequest([]byte("report")))
	assertCloudDocumentError(t, err, "cloud_api_error")
	if calls.Load() != 1 || escaped.Load() != 0 {
		t.Fatal("redirect forwarded upload bytes or credentials")
	}
	for _, response := range []string{`{`, `null`, `{}`, `{"id":123}`, `{"id":"https://untrusted.invalid/file"}`, `{"id":"123","messages":[]}`} {
		t.Run(response, func(t *testing.T) {
			var calls atomic.Int32
			client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) { calls.Add(1); _, _ = io.WriteString(w, response) })
			if response != `{"id":"123","messages":[]}` {
				_, err := client.uploadDocument(context.Background(), cloudTestConfig(), documentUploadRequest([]byte("report")))
				assertCloudDocumentError(t, err, "cloud_outcome_unknown")
			}
			body, err := cloudDocumentPayload(documentSendRequest())
			if err != nil {
				t.Fatal(err)
			}
			_, err = client.send(context.Background(), cloudTestConfig(), "+15551234567", body)
			assertCloudDocumentError(t, err, "cloud_outcome_unknown")
			want := int32(2)
			if response == `{"id":"123","messages":[]}` {
				want = 1
			}
			if calls.Load() != want {
				t.Fatalf("invalid acknowledgement caused retry: %d calls", calls.Load())
			}
		})
	}
}

func TestCloudDocumentPDFUploadPreservesBytes(t *testing.T) {
	data := []byte("%PDF-1.7\n%\xe2\xe3\xcf\xd3\n1 0 obj\n<< /Type /Catalog >>\nendobj\n%%EOF\n")
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		if err := r.ParseMultipartForm(4096); err != nil {
			t.Error(err)
			return
		}
		defer r.MultipartForm.RemoveAll()
		file, header, err := r.FormFile("file")
		if err != nil {
			t.Error(err)
			return
		}
		defer file.Close()
		got, err := io.ReadAll(file)
		if err != nil || !bytes.Equal(got, data) || header.Header.Get("Content-Type") != "application/pdf" {
			t.Error("PDF bytes or MIME changed")
		}
		_, _ = io.WriteString(w, `{"id":"7654321"}`)
	})
	req := CloudDocumentUploadRequest{DataBase64: base64.StdEncoding.EncodeToString(data), Mimetype: "application/pdf", FileName: "report.PDF"}
	uploaded, err := client.uploadDocument(context.Background(), cloudTestConfig(), req)
	if err != nil || uploaded.MediaID != "7654321" || uploaded.FileLength != len(data) || uploaded.FileName != "report.PDF" {
		t.Fatalf("valid PDF upload failed: %#v %v", uploaded, err)
	}
}

func TestCloudDocumentOfficeMIMEValidation(t *testing.T) {
	for _, tc := range []struct {
		extension, mimetype, entry string
	}{
		{"doc", "application/msword", ""},
		{"xls", "application/vnd.ms-excel", ""},
		{"ppt", "application/vnd.ms-powerpoint", ""},
		{"docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document", "word/document.xml"},
		{"xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", "xl/workbook.xml"},
		{"pptx", "application/vnd.openxmlformats-officedocument.presentationml.presentation", "ppt/presentation.xml"},
	} {
		t.Run(tc.extension, func(t *testing.T) {
			// These minimal signatures exercise the local MIME guard. Meta remains
			// responsible for validating complete Office documents after upload.
			data := []byte{0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1}
			if tc.entry != "" {
				var archive bytes.Buffer
				writer := zip.NewWriter(&archive)
				for _, name := range []string{"[Content_Types].xml", tc.entry} {
					file, err := writer.Create(name)
					if err != nil {
						t.Fatal(err)
					}
					if _, err := io.WriteString(file, "<test/>"); err != nil {
						t.Fatal(err)
					}
				}
				if err := writer.Close(); err != nil {
					t.Fatal(err)
				}
				data = archive.Bytes()
			}
			var calls atomic.Int32
			client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
				calls.Add(1)
				if err := r.ParseMultipartForm(4096); err != nil {
					t.Error(err)
					return
				}
				defer r.MultipartForm.RemoveAll()
				file, header, err := r.FormFile("file")
				if err != nil {
					t.Error(err)
					return
				}
				defer file.Close()
				got, err := io.ReadAll(file)
				if err != nil || !bytes.Equal(got, data) || header.Header.Get("Content-Type") != tc.mimetype {
					t.Error("Office upload changed MIME or file bytes")
				}
				_, _ = io.WriteString(w, `{"id":"1234567"}`)
			})
			req := CloudDocumentUploadRequest{DataBase64: base64.StdEncoding.EncodeToString(data), Mimetype: tc.mimetype, FileName: "report." + tc.extension}
			if _, err := client.uploadDocument(context.Background(), cloudTestConfig(), req); err != nil {
				t.Fatal(err)
			}
			req.DataBase64 = base64.StdEncoding.EncodeToString([]byte("plain text falsely declared as Office"))
			_, err := client.uploadDocument(context.Background(), cloudTestConfig(), req)
			assertCloudDocumentError(t, err, "cloud_unsupported_document")
			if calls.Load() != 1 {
				t.Fatal("invalid Office content reached the API")
			}
		})
	}
}

func TestCloudDocumentAcknowledgementsDoNotMistakeSecretSubstrings(t *testing.T) {
	config := cloudTestConfig()
	config.Cloud.VerifyToken = "1"
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		if strings.HasSuffix(r.URL.Path, "/media") {
			_, _ = io.WriteString(w, `{"id":"1234567"}`)
		} else {
			_, _ = io.WriteString(w, `{"messages":[{"id":"wamid.1234567"}]}`)
		}
	})
	if _, err := client.uploadDocument(context.Background(), config, documentUploadRequest([]byte("report"))); err != nil {
		t.Fatalf("legitimate media ID coinciding with a short secret substring rejected: %v", err)
	}
	body, err := cloudDocumentPayload(documentSendRequest())
	if err != nil {
		t.Fatal(err)
	}
	if _, err = client.send(context.Background(), config, "+15551234567", body); err != nil {
		t.Fatalf("legitimate message ID coinciding with a short secret substring rejected: %v", err)
	}
	config.Cloud.VerifyToken = "1234567"
	_, err = client.uploadDocument(context.Background(), config, documentUploadRequest([]byte("report")))
	assertCloudDocumentError(t, err, "cloud_outcome_unknown")
	if strings.Contains(err.Error(), config.Cloud.VerifyToken) {
		t.Fatal("exact credential echoed in acknowledgement error")
	}
}

func TestCloudDocumentGatewayFailureLeavesOutcomeUnknown(t *testing.T) {
	for _, response := range []string{"<html>Bad gateway</html>", `{}`, `{"error":"upstream unavailable"}`} {
		t.Run(response, func(t *testing.T) {
			var calls atomic.Int32
			client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
				calls.Add(1)
				w.WriteHeader(http.StatusBadGateway)
				_, _ = io.WriteString(w, response)
			})
			_, err := client.uploadDocument(context.Background(), cloudTestConfig(), documentUploadRequest([]byte("report")))
			assertCloudDocumentError(t, err, "cloud_outcome_unknown")
			if calls.Load() != 1 {
				t.Fatalf("ambiguous gateway error caused %d attempts", calls.Load())
			}
		})
	}
}
