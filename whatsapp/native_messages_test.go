package main

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"io"
	"math"
	"net/http"
	"net/url"
	"reflect"
	"strings"
	"testing"

	validate "github.com/santhosh-tekuri/jsonschema/v6"
	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/types"
)

func TestNativeMessageKeyParticipants(t *testing.T) {
	for _, tc := range []struct {
		chat        string
		fromMe      bool
		participant string
		valid       bool
	}{
		{"1234567@s.whatsapp.net", false, "", true}, {"1234567@g.us", false, "", false}, {"1234567@g.us", true, "", true}, {"1234567@g.us", false, "1234567:4@lid", true}, {"status@broadcast", true, "", false}, {"1234567@g.us", false, "1234567@g.us", false},
	} {
		t.Run(tc.chat+tc.participant+nativeBoolIndex(tc.fromMe), func(t *testing.T) {
			key, _, err := nativeKey(map[string]any{"chat_id": tc.chat, "message_id": "ABC", "from_me": tc.fromMe, "participant_id": tc.participant})
			if (err == nil) != tc.valid {
				t.Fatalf("valid=%v error=%v", tc.valid, err)
			}
			if err == nil && key.GetParticipant() != tc.participant {
				t.Fatal("participant namespace or device changed")
			}
		})
	}
	for _, id := range []string{"", "a b", "a\nb", strings.Repeat("x", 201), "a\x00b"} {
		if nativeMessageID(id) == nil {
			t.Errorf("accepted invalid message ID %q", id)
		}
	}
}

func TestNativePayloadsPreserveTextAndQuotes(t *testing.T) {
	chat := types.NewJID("1234567", types.DefaultUserServer)
	args := map[string]any{"text": "  hello 😀\n", "message_id": "original", "sender_id": "9876543@lid", "quoted_text": "quoted\n text"}
	payload, err := nativeMessagePayload(nil, "reply", args, chat)
	if err != nil {
		t.Fatal(err)
	}
	ext := payload.GetExtendedTextMessage()
	if ext.GetText() != args["text"] || ext.GetContextInfo().GetQuotedMessage().GetConversation() != args["quoted_text"] || ext.GetContextInfo().GetParticipant() != "9876543@lid" || ext.GetContextInfo().GetRemoteJID() != chat.String() {
		t.Fatal("reply changed supplied message or target")
	}
	if _, err = nativeMessagePayload(nil, "send_text", args, types.StatusBroadcastJID); err == nil {
		t.Fatal("generic send bypassed explicit status audience")
	}
	for _, text := range []string{"  ", "text\x00", strings.Repeat("é", 32769)} {
		if nativeText(text, "text", 65536, false) == nil {
			t.Fatal("accepted invalid text")
		}
	}
	for _, lat := range []float64{91, -91, math.NaN(), math.Inf(1)} {
		if _, err = nativeMessagePayload(nil, "send_location", map[string]any{"latitude": lat, "longitude": 0.0}, chat); err == nil {
			t.Fatal("accepted invalid coordinates")
		}
	}
	if _, err = nativeMessagePayload(nil, "send_contact", map[string]any{"display_name": "Name", "vcard": "unstructured text"}, chat); err == nil {
		t.Fatal("accepted invalid vCard")
	}
}

func TestNativeRangePreservesEveryKeyAndTimestamp(t *testing.T) {
	chat := types.NewJID("1234567", types.GroupServer)
	args := map[string]any{"last_message_timestamp_ms": json.Number("5001"), "last_system_message_timestamp_ms": float64(4000), "messages": []any{
		map[string]any{"key": map[string]any{"chat_id": chat.String(), "message_id": "first", "from_me": false, "participant_id": "7654321@lid"}, "timestamp_ms": float64(2001)},
		map[string]any{"key": map[string]any{"chat_id": chat.String(), "message_id": "second", "from_me": true}, "timestamp_ms": float64(3001)},
	}}
	value, err := nativeRange(args, chat)
	if err != nil {
		t.Fatal(err)
	}
	if value.GetLastMessageTimestamp() != 5 || value.GetLastSystemMessageTimestamp() != 4 || len(value.Messages) != 2 || value.Messages[0].GetTimestamp() != 2 || value.Messages[1].GetKey().GetID() != "second" {
		t.Fatal("range lost input")
	}
	patch, err := nativeChatPatch("clear", map[string]any{"message_range": args, "delete_starred": true, "delete_media": false}, chat)
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(patch.Mutations[0].Index, []string{"clearChat", chat.String(), "1", "0"}) || patch.Mutations[0].Version != 6 {
		t.Fatal("clear flags or protocol version changed")
	}
	patch, err = nativeChatPatch("archive", map[string]any{"message_range": args}, chat)
	if err != nil {
		t.Fatal(err)
	}
	if len(patch.Mutations[0].Value.ArchiveChatAction.MessageRange.Messages) != 2 {
		t.Fatal("archive collapsed supplied range")
	}
	args["last_message_timestamp_ms"] = float64(999)
	if _, err = nativeRange(args, chat); err == nil {
		t.Fatal("accepted subsecond timestamp")
	}
	args["last_message_timestamp_ms"] = float64(5000)
	if _, err = nativeRange(args, types.NewJID("other", types.GroupServer)); err == nil {
		t.Fatal("accepted cross-chat range")
	}
}

func TestNativePollOptionValidation(t *testing.T) {
	for _, options := range [][]string{{"one", "one"}, {"one", " "}, {strings.Repeat("x", 101), "two"}, make([]string, 13)} {
		if nativePollOptions(options, 2) == nil {
			t.Fatal("accepted invalid poll options")
		}
	}
	if err := nativePollOptions(nil, 0); err != nil {
		t.Fatal("empty vote must clear selection", err)
	}
}

func TestNativeMediaContentAndPayload(t *testing.T) {
	for _, kind := range []string{"image", "video", "audio", "document", "sticker"} {
		args := map[string]any{"media_type": kind, "data_base64": base64.StdEncoding.EncodeToString([]byte("encoded")), "duration_seconds": 12.0}
		data, thumb, err := nativeMediaContent(args, 8)
		if err != nil {
			t.Fatal(err)
		}
		payload := nativeMediaMessage(args, whatsmeow.UploadResponse{MediaKey: []byte("key"), FileLength: uint64(len(data)), DirectPath: "/mms/example"}, thumb)
		if payload == nil {
			t.Fatal("missing media payload")
		}
		if kind == "audio" && payload.AudioMessage.GetSeconds() != 12 {
			t.Fatal("audio duration lost")
		}
		if kind == "sticker" && payload.StickerMessage.GetMimetype() != "image/webp" {
			t.Fatal("sticker MIME changed")
		}
		args["data_base64"] = base64.StdEncoding.EncodeToString([]byte("too large"))
		if _, _, err = nativeMediaContent(args, 8); err == nil {
			t.Fatal("accepted oversized media")
		}
	}
	for _, args := range []map[string]any{{"media_type": "image", "voice_note": true}, {"media_type": "audio", "caption": ""}, {"media_type": "image", "mimetype": "video/mp4"}, {"media_type": "audio", "gif_playback": true}} {
		args["data_base64"] = "YQ=="
		if _, _, err := nativeMediaContent(args, 8); err == nil {
			t.Fatal("accepted incompatible media options")
		}
	}
}

func TestNativeMediaDownloadValidation(t *testing.T) {
	hash := base64.StdEncoding.EncodeToString(make([]byte, 32))
	args := map[string]any{"direct_path": "/mms/file?abc=1", "file_length": float64(8), "file_sha256_base64": hash}
	if _, _, _, _, _, err := nativeDownloadArgs(args, 8); err != nil {
		t.Fatal(err)
	}
	args["media_key_base64"] = hash
	if _, _, _, _, _, err := nativeDownloadArgs(args, 8); err == nil {
		t.Fatal("accepted key without encrypted hash")
	}
	args["file_enc_sha256_base64"] = hash
	for _, path := range []string{"https://example.com/file", "/mms/white space", "/v/fragment#x", "/mms/back\\slash", "/mms/control\x01"} {
		args["direct_path"] = path
		if _, _, _, _, _, err := nativeDownloadArgs(args, 8); err == nil {
			t.Errorf("accepted path %q", path)
		}
	}
	args["direct_path"] = "/v/file"
	args["file_length"] = float64(9)
	if _, _, _, _, _, err := nativeDownloadArgs(args, 8); err == nil {
		t.Fatal("accepted oversized download")
	}
}

type nativeRoundTripFunc func(*http.Request) (*http.Response, error)

func (f nativeRoundTripFunc) RoundTrip(r *http.Request) (*http.Response, error) { return f(r) }
func TestNativeCDNTransportBoundsUnknownLengthAndRejectsHosts(t *testing.T) {
	calls := 0
	transport := nativeMediaTransport{maxBytes: func() int { return 8 }, base: nativeRoundTripFunc(func(*http.Request) (*http.Response, error) {
		calls++
		return &http.Response{StatusCode: 200, ContentLength: -1, Body: io.NopCloser(bytes.NewReader(make([]byte, 100)))}, nil
	})}
	request, _ := http.NewRequestWithContext(context.WithValue(context.Background(), nativeMediaLimitKey{}, int64(8)), http.MethodGet, "https://mmg.whatsapp.net/mms/file", nil)
	response, err := transport.RoundTrip(request)
	if err != nil {
		t.Fatal(err)
	}
	data, err := io.ReadAll(response.Body)
	response.Body.Close()
	if err == nil || len(data) > 8 {
		t.Fatalf("unbounded read: %d, %v", len(data), err)
	}
	for _, raw := range []string{"http://mmg.whatsapp.net/mms/file", "https://whatsapp.net.evil.test/file", "https://127.0.0.1/file", "https://mmg.whatsapp.net:8443/file", "https://name:secret@mmg.whatsapp.net/file"} {
		u, _ := url.Parse(raw)
		if nativeMediaURL(u) {
			t.Errorf("accepted non-CDN URL %s", raw)
		}
		request.URL = u
		if _, err = transport.RoundTrip(request); err == nil {
			t.Fatal("unsafe URL reached transport")
		}
	}
	if calls != 1 {
		t.Fatal("rejected URL reached network")
	}
	for _, n := range []int{0, 8, 9} {
		body := &nativeBoundedBody{ReadCloser: io.NopCloser(bytes.NewReader(make([]byte, n))), remaining: 8}
		data, err := io.ReadAll(body)
		if (err == nil) != (n <= 8) || len(data) > 8 {
			t.Fatalf("bound %d failed", n)
		}
	}
}

func TestNativeNewsletterMissingMetadataIsNull(t *testing.T) {
	message, err := nativeNewsletterMessage(&types.NewsletterMessage{MessageID: "ID"})
	if err != nil {
		t.Fatal(err)
	}
	for _, key := range []string{"is_sender", "votes", "admin_profile", "original_timestamp"} {
		if message[key] != nil {
			t.Errorf("fabricated %s", key)
		}
	}
	meta := nativeNewsletterMetadata(&types.NewsletterMetadata{ID: types.NewJID("1234", types.NewsletterServer)})
	if meta["muted"] != nil || meta["role"] != nil || meta["follower_activity_muted"] != nil {
		t.Fatal("fabricated viewer metadata")
	}
}

func TestNativeMediaErrorsDoNotExposeAuthorizationURLs(t *testing.T) {
	err := &url.Error{Op: "Post", URL: "https://mmg.whatsapp.net/mms/upload?auth=private-token", Err: errors.New("connection refused")}
	if result := nativeMediaError("upload", err).Error(); strings.Contains(result, "private-token") || strings.Contains(result, "auth=") {
		t.Fatal("media error exposed authorization URL")
	}
	if !errors.Is(nativeMediaError("download", context.DeadlineExceeded), context.DeadlineExceeded) {
		t.Fatal("timeout identity lost")
	}
}

func TestNativeAcceptedSendArchiveFailureWarnsAgainstResend(t *testing.T) {
	chat := types.NewJID("1234567", types.DefaultUserServer)
	err := nativeRecordSent(&Manager{}, "missing", nil, whatsmeow.SendResponse{ID: "ACCEPTED-ID", Sender: chat}, chat, "private body", "text")
	if err == nil || !strings.Contains(err.Error(), "message ACCEPTED-ID was accepted") || !strings.Contains(err.Error(), "do not resend automatically") || strings.Contains(err.Error(), "private body") {
		t.Fatalf("unsafe or ambiguous accepted-send failure: %v", err)
	}
}

func TestNativeRegistrationAndResponseContracts(t *testing.T) {
	r := &Registry{handlers: make(map[string]operationHandler)}
	registerMessaging(r)
	for _, id := range []string{"status::audience", "status::send_text", "status::send_media", "status::revoke", "calls::terminate", "chats::unsubscribe_presence", "newsletters::vote_poll", "newsletters::delete"} {
		if _, ok := r.handlers["whatsapp::"+id]; ok {
			t.Fatalf("unsupported handler registered: %s", id)
		}
	}
	message, err := nativeNewsletterMessage(&types.NewsletterMessage{MessageID: "ID"})
	if err != nil {
		t.Fatal(err)
	}
	for _, tc := range []struct {
		id    string
		value any
	}{
		{"newsletters::messages", []any{message}},
		{"newsletters::get", nativeNewsletterMetadata(&types.NewsletterMetadata{ID: types.NewJID("1234", types.NewsletterServer)})},
		{"messages::send_text", nativeSent(whatsmeow.SendResponse{ID: "ID"}, types.NewJID("1234567", types.DefaultUserServer))},
	} {
		t.Run(tc.id, func(t *testing.T) {
			compiler := validate.NewCompiler()
			uri := "https://whatsapp.invalid/native-response.json"
			if err := compiler.AddResource(uri, catalogEntries["whatsapp::"+tc.id].ResponseFormat); err != nil {
				t.Fatal(err)
			}
			schema, err := compiler.Compile(uri)
			if err != nil {
				t.Fatal(err)
			}
			encoded, err := json.Marshal(tc.value)
			if err != nil {
				t.Fatal(err)
			}
			var wire any
			if err = json.Unmarshal(encoded, &wire); err != nil {
				t.Fatal(err)
			}
			if err = schema.Validate(wire); err != nil {
				t.Fatal(err)
			}
		})
	}
}
