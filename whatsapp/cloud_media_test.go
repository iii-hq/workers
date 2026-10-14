package main

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"image"
	"image/color"
	"image/png"
	"io"
	"net"
	"net/http"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func cloudMediaTestPNG(t *testing.T) []byte {
	t.Helper()
	var out bytes.Buffer
	picture := image.NewNRGBA(image.Rect(0, 0, 2, 2))
	picture.SetNRGBA(0, 0, color.NRGBA{R: 100, A: 200})
	if err := png.Encode(&out, picture); err != nil {
		t.Fatal(err)
	}
	return out.Bytes()
}

func TestCloudMediaUploadMultipart(t *testing.T) {
	data := cloudMediaTestPNG(t)
	var calls atomic.Int32
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		if r.Method != http.MethodPost || r.URL.Path != "/v26.0/456/media" || r.Header.Get("Authorization") != "Bearer test-access-secret" {
			t.Error("wrong upload route or authorization")
		}
		if err := r.ParseMultipartForm(1 << 20); err != nil {
			t.Fatal(err)
		}
		file, header, err := r.FormFile("file")
		if err != nil {
			t.Fatal(err)
		}
		content, _ := io.ReadAll(file)
		if header.Filename != "photo.png" || header.Header.Get("Content-Type") != "image/png" {
			t.Error("incorrect MIME or filename")
		}
		if r.FormValue("messaging_product") != "whatsapp" || r.FormValue("type") != "image/png" || !bytes.Equal(content, data) || len(r.MultipartForm.Value) != 2 || len(r.MultipartForm.File) != 1 {
			t.Error("incorrect multipart fields")
		}
		_, _ = io.WriteString(w, `{"id":"987"}`)
	})
	got, err := client.uploadMedia(context.Background(), cloudTestConfig(), CloudMediaUploadRequest{MediaType: "image", DataBase64: base64.StdEncoding.EncodeToString(data), Mimetype: "image/png", FileName: "photo.png"})
	if err != nil || got.MediaID != "987" || got.MediaType != "image" || got.FileLength != len(data) || calls.Load() != 1 {
		t.Fatalf("unexpected upload: %+v %v calls=%d", got, err, calls.Load())
	}
}

func TestCloudMediaUploadRejectsUnsafeInputBeforeHTTP(t *testing.T) {
	data := cloudMediaTestPNG(t)
	var calls atomic.Int32
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) { calls.Add(1); _, _ = io.WriteString(w, `{"id":"987"}`) })
	for _, tt := range []struct {
		name, code string
		change     func(*CloudMediaUploadRequest, *Config)
	}{
		{"missing credential", "cloud_not_configured", func(r *CloudMediaUploadRequest, c *Config) { c.Cloud.AccessToken = "" }},
		{"kind mismatch", "cloud_unsupported_media", func(r *CloudMediaUploadRequest, c *Config) { r.MediaType = "video" }},
		{"MIME mismatch", "cloud_unsupported_media", func(r *CloudMediaUploadRequest, c *Config) { r.Mimetype = "image/jpeg" }},
		{"spoofed MIME", "cloud_unsupported_media", func(r *CloudMediaUploadRequest, c *Config) {
			r.DataBase64 = base64.StdEncoding.EncodeToString([]byte("not an image"))
		}},
		{"host path", "invalid_request", func(r *CloudMediaUploadRequest, c *Config) { r.FileName = "/tmp/photo.png" }},
		{"control filename", "invalid_request", func(r *CloudMediaUploadRequest, c *Config) { r.FileName = "photo\r\n.png" }},
		{"size limit", "cloud_media_too_large", func(r *CloudMediaUploadRequest, c *Config) { c.MaxMediaBytes = len(data) - 1 }},
		{"invalid base64", "invalid_request", func(r *CloudMediaUploadRequest, c *Config) { r.DataBase64 = "not-base64" }},
		{"Markdown", "cloud_unsupported_document", func(r *CloudMediaUploadRequest, c *Config) {
			r.MediaType = "document"
			r.Mimetype = "text/markdown"
			r.FileName = "report.md"
		}},
	} {
		t.Run(tt.name, func(t *testing.T) {
			config := cloudTestConfig()
			req := CloudMediaUploadRequest{MediaType: "image", DataBase64: base64.StdEncoding.EncodeToString(data), Mimetype: "image/png", FileName: "photo.png"}
			tt.change(&req, &config)
			_, err := client.uploadMedia(context.Background(), config, req)
			assertCloudDocumentError(t, err, tt.code)
		})
	}
	if calls.Load() != 0 {
		t.Fatalf("invalid uploads reached HTTP: %d", calls.Load())
	}
}

type cloudMediaTestTransport func(*http.Request) (*http.Response, error)

func (f cloudMediaTestTransport) RoundTrip(r *http.Request) (*http.Response, error) { return f(r) }

func cloudMediaHTTPResponse(code int, contentType string, body []byte) *http.Response {
	return &http.Response{StatusCode: code, Header: http.Header{"Content-Type": []string{contentType}}, Body: io.NopCloser(bytes.NewReader(body)), ContentLength: int64(len(body))}
}

func TestCloudMediaMetadataDownloadAndDelete(t *testing.T) {
	data := cloudMediaTestPNG(t)
	sum := sha256.Sum256(data)
	digest := hex.EncodeToString(sum[:])
	var metadata, downloads, deleted int
	client := newCloudClient()
	client.http.Transport = cloudMediaTestTransport(func(r *http.Request) (*http.Response, error) {
		if r.Header.Get("Authorization") != "Bearer test-access-secret" {
			t.Error("missing auth")
		}
		if r.URL.Host == "graph.facebook.com" {
			if r.URL.Path != "/v26.0/987" || r.URL.Query().Get("phone_number_id") != "456" {
				t.Error("media request must be phone scoped")
			}
			if r.Method == http.MethodDelete {
				deleted++
				return cloudMediaHTTPResponse(200, "application/json", []byte(`{"success":true}`)), nil
			}
			metadata++
			return cloudMediaHTTPResponse(200, "application/json", []byte(fmt.Sprintf(`{"id":"987","mime_type":"image/png","file_size":"%d","sha256":"%s","url":"https://lookaside.fbsbx.com/whatsapp_business/attachments/?token=opaque"}`, len(data), digest))), nil
		}
		if r.URL.Host != "lookaside.fbsbx.com" || r.Method != http.MethodGet {
			t.Error("unexpected media request")
		}
		downloads++
		return cloudMediaHTTPResponse(200, "image/png", data), nil
	})
	got, err := client.getMedia(context.Background(), cloudTestConfig(), "987")
	if err != nil || got.MediaID != "987" || got.FileLength != len(data) || got.SHA256 != digest {
		t.Fatalf("metadata: %+v %v", got, err)
	}
	encoded, _ := json.Marshal(got)
	if strings.Contains(string(encoded), "url") || strings.Contains(string(encoded), "token") {
		t.Fatal("metadata leaked authenticated URL")
	}
	download, err := client.downloadMedia(context.Background(), cloudTestConfig(), "987")
	if err != nil || download.DataBase64 != base64.StdEncoding.EncodeToString(data) || download.Mimetype != "image/png" {
		t.Fatalf("download: %+v %v", download, err)
	}
	result, err := client.deleteMedia(context.Background(), cloudTestConfig(), "987")
	if err != nil || !result.Deleted || result.MediaID != "987" {
		t.Fatalf("delete: %+v %v", result, err)
	}
	if metadata != 2 || downloads != 1 || deleted != 1 {
		t.Fatalf("unexpected requests: %d %d %d", metadata, downloads, deleted)
	}
}

func TestCloudMediaDownloadRejectsUnsafeURLsAndMetadata(t *testing.T) {
	for _, tt := range []struct{ name, url, mime, size, code string }{
		{"http", "http://lookaside.fbsbx.com/media", "image/png", "100", "cloud_unsafe_media_url"},
		{"host", "https://example.com/media", "image/png", "100", "cloud_unsafe_media_url"},
		{"suffix bypass", "https://lookaside.fbsbx.com.attacker.test/media", "image/png", "100", "cloud_unsafe_media_url"},
		{"private IP", "https://127.0.0.1/media", "image/png", "100", "cloud_unsafe_media_url"},
		{"userinfo", "https://user@lookaside.fbsbx.com/media", "image/png", "100", "cloud_unsafe_media_url"},
		{"port", "https://lookaside.fbsbx.com:8443/media", "image/png", "100", "cloud_unsafe_media_url"},
		{"MIME", "https://lookaside.fbsbx.com/media", "application/x-executable", "100", "cloud_unsupported_media"},
		{"oversize", "https://lookaside.fbsbx.com/media", "image/png", "5000001", "cloud_media_too_large"},
		{"invalid size", "https://lookaside.fbsbx.com/media", "image/png", "-1", "cloud_invalid_response"},
	} {
		t.Run(tt.name, func(t *testing.T) {
			calls := 0
			client := newCloudClient()
			client.http.Transport = cloudMediaTestTransport(func(r *http.Request) (*http.Response, error) {
				calls++
				return cloudMediaHTTPResponse(200, "application/json", []byte(fmt.Sprintf(`{"id":"987","mime_type":%q,"file_size":%q,"sha256":"%s","url":%q}`, tt.mime, tt.size, strings.Repeat("a", 64), tt.url))), nil
			})
			_, err := client.downloadMedia(context.Background(), cloudTestConfig(), "987")
			assertCloudDocumentError(t, err, tt.code)
			if calls != 1 {
				t.Fatalf("unsafe metadata caused extra request: %d", calls)
			}
		})
	}
	for _, ip := range []string{"127.0.0.1", "10.1.2.3", "169.254.169.254", "100.64.0.1", "192.0.2.1", "198.18.0.1", "::1", "fc00::1", "fe80::1", "2001:db8::1"} {
		if cloudPublicMediaIP(net.ParseIP(ip)) {
			t.Errorf("reserved address accepted: %s", ip)
		}
	}
}

func TestCloudMediaDownloadNoRedirectRetryOrSecretLeak(t *testing.T) {
	data := cloudMediaTestPNG(t)
	sum := sha256.Sum256(data)
	for _, tt := range []struct {
		name        string
		code        int
		body        []byte
		contentType string
		want        string
	}{
		{"redirect", 302, []byte("test-access-secret"), "text/plain", "cloud_download_error"},
		{"expired", 404, []byte("test-access-secret"), "text/plain", "cloud_download_error"},
		{"MIME mismatch", 200, data, "text/plain", "cloud_unsupported_media"},
		{"truncated", 200, data[:len(data)-1], "image/png", "cloud_invalid_response"},
		{"altered", 200, bytes.Repeat([]byte("x"), len(data)), "image/png", "cloud_invalid_response"},
	} {
		t.Run(tt.name, func(t *testing.T) {
			calls := 0
			client := newCloudClient()
			client.http.Transport = cloudMediaTestTransport(func(r *http.Request) (*http.Response, error) {
				calls++
				if calls == 1 {
					return cloudMediaHTTPResponse(200, "application/json", []byte(fmt.Sprintf(`{"id":"987","mime_type":"image/png","file_size":%d,"sha256":"%x","url":"https://lookaside.fbsbx.com/media?token=test-access-secret"}`, len(data), sum))), nil
				}
				response := cloudMediaHTTPResponse(tt.code, tt.contentType, tt.body)
				response.Header.Set("Location", "https://example.com/leak")
				return response, nil
			})
			_, err := client.downloadMedia(context.Background(), cloudTestConfig(), "987")
			assertCloudDocumentError(t, err, tt.want)
			if calls != 2 {
				t.Fatalf("download followed redirect or retried: %d", calls)
			}
		})
	}
}

func TestCloudMediaSendPayloadsAndArchive(t *testing.T) {
	config := cloudTestConfig()
	var calls atomic.Int32
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		if r.Method != http.MethodPost || r.URL.Path != "/v26.0/456/messages" || r.Header.Get("Authorization") != "Bearer test-access-secret" {
			t.Error("incorrect route or authorization")
		}
		var payload map[string]any
		if err := json.NewDecoder(r.Body).Decode(&payload); err != nil {
			t.Error(err)
			return
		}
		kind, _ := payload["type"].(string)
		media, ok := payload[kind].(map[string]any)
		if !ok || media["id"] != "987" || payload["to"] != "+15551234567" || payload["recipient_type"] != "individual" || payload["messaging_product"] != "whatsapp" {
			t.Errorf("incorrect media payload: %#v", payload)
		}
		if payload["context"].(map[string]any)["message_id"] != "wamid.original" || payload["recipient_identity_key_hash"] != "identity-hash" {
			t.Error("missing quoted reply or identity protection")
		}
		if kind == "image" || kind == "video" {
			if media["caption"] != "Caption" {
				t.Error("missing caption")
			}
		} else if _, exists := media["caption"]; exists {
			t.Error("audio/sticker caption sent")
		}
		_, _ = fmt.Fprintf(w, `{"messages":[{"id":"wamid.%s","message_status":"accepted"}]}`, kind)
	})
	registry := cloudTestRegistry(t, client, config, registerCloudMedia)
	for _, kind := range []string{"image", "audio", "video", "sticker"} {
		input := map[string]any{"chat_id": "+15551234567", "media_id": "987", "reply_to_message_id": "wamid.original", "recipient_identity_key_hash": "identity-hash"}
		if kind == "image" || kind == "video" {
			input["caption"] = "Caption"
		}
		result, err := registry.Invoke(context.Background(), "whatsapp::cloud::messages::send_"+kind, input)
		if err != nil {
			t.Fatal(err)
		}
		sent, ok := result.(CloudSentMessage)
		if !ok || sent.MessageID != "wamid."+kind || sent.Status != "accepted" {
			t.Fatalf("unexpected acknowledgement: %+v", result)
		}
	}
	if calls.Load() != 4 {
		t.Fatalf("unexpected send count %d", calls.Load())
	}
	inbox, err := registry.Manager.CloudInbox(config)
	if err != nil {
		t.Fatal(err)
	}
	page, err := inbox.ListMessages("cloud:+15551234567", "", 10)
	if err != nil || len(page.Messages) != 4 {
		t.Fatalf("archive: %+v %v", page, err)
	}
	for _, message := range page.Messages {
		if message.Status == nil || *message.Status != "submitted" {
			t.Error("API acceptance must not imply delivery")
		}
	}
}

func TestCloudMediaSendValidationAndFailures(t *testing.T) {
	for _, tt := range []struct {
		name, kind string
		req        CloudMediaMessageRequest
	}{
		{"native recipient", "image", CloudMediaMessageRequest{ChatID: "123@g.us", MediaID: "987"}},
		{"media path", "image", CloudMediaMessageRequest{ChatID: "+15551234567", MediaID: "../987"}},
		{"audio caption", "audio", CloudMediaMessageRequest{ChatID: "+15551234567", MediaID: "987", Caption: cloudMediaStringPointer("caption")}},
		{"sticker caption", "sticker", CloudMediaMessageRequest{ChatID: "+15551234567", MediaID: "987", Caption: cloudMediaStringPointer("caption")}},
		{"caption too long", "image", CloudMediaMessageRequest{ChatID: "+15551234567", MediaID: "987", Caption: cloudMediaStringPointer(strings.Repeat("a", 1025))}},
		{"invalid reply", "video", CloudMediaMessageRequest{ChatID: "+15551234567", MediaID: "987", ReplyToMessageID: "bad\nreply"}},
	} {
		t.Run(tt.name, func(t *testing.T) {
			_, err := cloudMediaMessagePayload(tt.kind, tt.req)
			assertCloudDocumentError(t, err, "invalid_request")
		})
	}
	for _, tt := range []struct {
		name           string
		status         int
		response, want string
	}{
		{"window closed", 400, `{"error":{"code":131047,"message":"test-access-secret"}}`, "cloud_window_closed"},
		{"Meta error", 400, `{"error":{"code":100,"message":"test-access-secret"}}`, "cloud_api_error"},
		{"missing acknowledgement", 200, `{}`, "cloud_outcome_unknown"},
		{"unreadable acknowledgement", 200, `test-access-secret`, "cloud_outcome_unknown"},
	} {
		t.Run(tt.name, func(t *testing.T) {
			calls := 0
			client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
				calls++
				w.WriteHeader(tt.status)
				_, _ = io.WriteString(w, tt.response)
			})
			body, err := cloudMediaMessagePayload("image", CloudMediaMessageRequest{ChatID: "+15551234567", MediaID: "987"})
			if err != nil {
				t.Fatal(err)
			}
			_, err = client.send(context.Background(), cloudTestConfig(), "+15551234567", body)
			assertCloudDocumentError(t, err, tt.want)
			if calls != 1 {
				t.Fatalf("send retried: %d", calls)
			}
		})
	}
}

func cloudMediaStringPointer(value string) *string { return &value }

func TestCloudMediaUploadAndDeleteTimeoutAreAmbiguous(t *testing.T) {
	for _, operation := range []string{"upload", "delete"} {
		t.Run(operation, func(t *testing.T) {
			var calls atomic.Int32
			started := make(chan struct{})
			release := make(chan struct{})
			defer close(release)
			client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
				calls.Add(1)
				close(started)
				select {
				case <-r.Context().Done():
				case <-release:
				}
			})
			ctx, cancel := context.WithCancel(context.Background())
			t.Cleanup(cancel)
			go func() {
				select {
				case <-started:
					cancel()
				case <-time.After(time.Second):
					cancel()
				}
			}()
			var err error
			if operation == "upload" {
				_, err = client.uploadMedia(ctx, cloudTestConfig(), CloudMediaUploadRequest{MediaType: "image", Mimetype: "image/png", FileName: "photo.png", DataBase64: base64.StdEncoding.EncodeToString(cloudMediaTestPNG(t))})
			} else {
				_, err = client.deleteMedia(ctx, cloudTestConfig(), "987")
			}
			assertCloudDocumentError(t, err, "cloud_outcome_unknown")
			if calls.Load() != 1 {
				t.Fatalf("operation retried: %d", calls.Load())
			}
		})
	}
}

func cloudMediaTestBox(kind string, body []byte) []byte {
	box := make([]byte, 8+len(body))
	binary.BigEndian.PutUint32(box[:4], uint32(len(box)))
	copy(box[4:8], kind)
	copy(box[8:], body)
	return box
}

func cloudMediaTestMP4(codecs ...string) []byte {
	tracks := []byte{}
	for _, codec := range codecs {
		length := 28
		if codec == "avc1" || codec == "avc3" {
			length = 78
		}
		entry := cloudMediaTestBox(codec, make([]byte, length))
		stsd := append([]byte{0, 0, 0, 0, 0, 0, 0, 1}, entry...)
		track := cloudMediaTestBox("trak", cloudMediaTestBox("mdia", cloudMediaTestBox("minf", cloudMediaTestBox("stbl", cloudMediaTestBox("stsd", stsd)))))
		tracks = append(tracks, track...)
	}
	return append(cloudMediaTestBox("ftyp", []byte("isom\x00\x00\x00\x00isom")), cloudMediaTestBox("moov", tracks)...)
}

func cloudMediaTestWebP(animated bool, padding int) []byte {
	chunk := func(kind string, body []byte) []byte {
		result := make([]byte, 8+len(body)+(len(body)&1))
		copy(result, kind)
		binary.LittleEndian.PutUint32(result[4:8], uint32(len(body)))
		copy(result[8:], body)
		return result
	}
	body := []byte("WEBP")
	if animated {
		body = append(body, chunk("VP8X", []byte{2, 0, 0, 0, 0, 0, 0, 0, 0, 0})...)
		body = append(body, chunk("ANIM", make([]byte, 6))...)
		body = append(body, chunk("ANMF", make([]byte, 17))...)
	} else {
		body = append(body, chunk("VP8L", []byte{0x2f, 0, 0, 0, 0})...)
	}
	if padding > 0 {
		body = append(body, chunk("JUNK", make([]byte, padding))...)
	}
	result := make([]byte, 8+len(body))
	copy(result, "RIFF")
	binary.LittleEndian.PutUint32(result[4:8], uint32(len(body)))
	copy(result[8:], body)
	return result
}

func TestCloudMediaContainerAndCodecValidation(t *testing.T) {
	ogg := append([]byte{'O', 'g', 'g', 'S', 0, 2}, make([]byte, 20)...)
	ogg = append(ogg, 1, 19)
	opus := make([]byte, 19)
	copy(opus, "OpusHead")
	opus[8] = 1
	opus[9] = 1
	ogg = append(ogg, opus...)
	for _, tt := range []struct {
		name, mime string
		data       []byte
		valid      bool
		limit      int
	}{
		{"AAC", "audio/aac", []byte{0xff, 0xf1, 0x50, 0x80, 0, 0, 0, 0}, true, 16_000_000},
		{"AMR", "audio/amr", []byte("#!AMR\nframe"), true, 16_000_000},
		{"MP3", "audio/mpeg", []byte{0xff, 0xfb, 0x90, 0x64}, true, 16_000_000},
		{"Opus mono", "audio/ogg", ogg, true, 16_000_000},
		{"OGG no Opus", "audio/ogg", []byte("OggSnot-opus"), false, 0},
		{"MP4 audio", "audio/mp4", cloudMediaTestMP4("mp4a"), true, 16_000_000},
		{"MP4 video", "video/mp4", cloudMediaTestMP4("avc1", "mp4a"), true, 16_000_000},
		{"3GPP video", "video/3gpp", cloudMediaTestMP4("avc1"), true, 16_000_000},
		{"HEVC unsupported", "video/mp4", cloudMediaTestMP4("hvc1"), false, 0},
		{"multiple audio tracks", "video/mp4", cloudMediaTestMP4("avc1", "mp4a", "mp4a"), false, 0},
		{"video disguised as audio", "audio/mp4", cloudMediaTestMP4("avc1", "mp4a"), false, 0},
		{"static sticker", "image/webp", cloudMediaTestWebP(false, 0), true, 100_000},
		{"animated sticker", "image/webp", cloudMediaTestWebP(true, 0), true, 500_000},
		{"invalid sticker", "image/webp", []byte("RIFFforgedWEBP"), false, 0},
	} {
		t.Run(tt.name, func(t *testing.T) {
			limit, err := cloudMediaContent(tt.data, "file", tt.mime)
			if (err == nil) != tt.valid || limit != tt.limit {
				t.Fatalf("valid=%t limit=%d error=%v", tt.valid, limit, err)
			}
		})
	}
	stereo := bytes.Clone(ogg)
	stereo[28+9] = 2
	if _, err := cloudMediaContent(stereo, "voice.ogg", "audio/ogg"); err == nil {
		t.Fatal("stereo Opus must not be accepted")
	}
}

func TestCloudMediaStaticStickerLimitBeforeHTTP(t *testing.T) {
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) { t.Error("oversized static sticker reached Meta") })
	_, err := client.uploadMedia(context.Background(), cloudTestConfig(), CloudMediaUploadRequest{MediaType: "sticker", DataBase64: base64.StdEncoding.EncodeToString(cloudMediaTestWebP(false, 100_000)), Mimetype: "image/webp", FileName: "sticker.webp"})
	assertCloudDocumentError(t, err, "cloud_media_too_large")
}

func cloudMediaTestOgg(duration int) []byte {
	page := func(flags byte, sequence uint32, granule uint64, payload []byte) []byte {
		data := make([]byte, 28+len(payload))
		copy(data, "OggS")
		data[5] = flags
		binary.LittleEndian.PutUint64(data[6:14], granule)
		binary.LittleEndian.PutUint32(data[14:18], 1)
		binary.LittleEndian.PutUint32(data[18:22], sequence)
		data[26] = 1
		data[27] = byte(len(payload))
		copy(data[28:], payload)
		return data
	}
	header := make([]byte, 19)
	copy(header, "OpusHead")
	header[8] = 1
	header[9] = 1
	return append(page(2, 0, 0, header), page(4, 1, uint64(duration*48000), []byte{0xf8, 0xff, 0xfe})...)
}

func TestCloudMediaVoicemailUploadUseCaseAndDuration(t *testing.T) {
	calls := 0
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		calls++
		if err := r.ParseMultipartForm(1 << 20); err != nil {
			t.Error(err)
			return
		}
		if r.FormValue("use_case") != "call_voicemail_announcement" || r.FormValue("description") != "Welcome" || r.FormValue("type") != "audio/ogg" {
			t.Errorf("wrong voicemail multipart fields: %#v", r.MultipartForm.Value)
		}
		_, _ = io.WriteString(w, `{"id":"987"}`)
	})
	request := CloudVoicemailUploadRequest{DataBase64: base64.StdEncoding.EncodeToString(cloudMediaTestOgg(59)), Mimetype: "audio/ogg", FileName: "welcome.ogg", Description: "Welcome"}
	result, err := client.uploadVoicemail(context.Background(), cloudTestConfig(), request)
	if err != nil || result.UseCase != "call_voicemail_announcement" || result.MediaID != "987" {
		t.Fatalf("result=%+v error=%v", result, err)
	}
	request.DataBase64 = base64.StdEncoding.EncodeToString(cloudMediaTestOgg(60))
	_, err = client.uploadVoicemail(context.Background(), cloudTestConfig(), request)
	assertCloudDocumentError(t, err, "cloud_unsupported_media")
	if calls != 1 {
		t.Error("invalid voicemail reached upload")
	}
}
