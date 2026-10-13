package main

import (
	"context"
	"encoding/base64"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
	"time"
	"unicode"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"google.golang.org/protobuf/proto"
)

type nativeMediaLimitKey struct{}
type nativeMediaTransport struct {
	base     http.RoundTripper
	maxBytes func(context.Context) (int, error)
}
type nativeBoundedBody struct {
	io.ReadCloser
	remaining int64
}

func (b *nativeBoundedBody) Read(p []byte) (int, error) {
	if len(p) == 0 {
		return 0, nil
	}
	if b.remaining <= 0 {
		var extra [1]byte
		n, err := b.ReadCloser.Read(extra[:])
		if n > 0 {
			return 0, errors.New("media response exceeds configured byte limit")
		}
		return 0, err
	}
	if int64(len(p)) > b.remaining {
		p = p[:b.remaining]
	}
	n, err := b.ReadCloser.Read(p)
	b.remaining -= int64(n)
	return n, err
}
func nativeMediaURL(u *url.URL) bool {
	if u.Scheme != "https" || u.User != nil || u.Port() != "" && u.Port() != "443" {
		return false
	}
	host := strings.ToLower(u.Hostname())
	for _, domain := range []string{"whatsapp.net", "whatsapp.com", "fbcdn.net"} {
		if host == domain || strings.HasSuffix(host, "."+domain) {
			return true
		}
	}
	return false
}
func (t nativeMediaTransport) RoundTrip(request *http.Request) (*http.Response, error) {
	if !nativeMediaURL(request.URL) {
		return nil, errors.New("media requests require an HTTPS WhatsApp CDN host")
	}
	if err := request.Context().Err(); err != nil {
		return nil, err
	}
	limit := int64(1024 * 1024)
	if request.Method == http.MethodGet {
		if requested, ok := request.Context().Value(nativeMediaLimitKey{}).(int64); ok {
			limit = requested
		} else {
			maxBytes, err := t.maxBytes(request.Context())
			if err != nil {
				return nil, err
			}
			limit = int64(maxBytes) + 26
		}
	}
	if limit < 1 {
		return nil, errors.New("media response byte limit must be positive")
	}
	response, err := t.base.RoundTrip(request)
	if err != nil {
		return nil, err
	}
	if response.ContentLength > limit {
		response.Body.Close()
		return nil, errors.New("media response exceeds configured byte limit")
	}
	response.Body = &nativeBoundedBody{ReadCloser: response.Body, remaining: limit}
	return response, nil
}

// Install once when the client is created. The limit callback supports config reloads.
func configureNativeMediaHTTP(client *whatsmeow.Client, maxBytes func(context.Context) (int, error)) {
	transport := http.DefaultTransport.(*http.Transport).Clone()
	client.SetMediaHTTPClient(&http.Client{Transport: nativeMediaTransport{base: transport, maxBytes: maxBytes}, Timeout: 2 * time.Minute, CheckRedirect: func(req *http.Request, via []*http.Request) error {
		if len(via) >= 5 || !nativeMediaURL(req.URL) {
			return errors.New("media redirect is not allowed")
		}
		return nil
	}})
}

func nativeMediaType(kind string) (whatsmeow.MediaType, string, error) {
	switch kind {
	case "image", "sticker":
		return whatsmeow.MediaImage, "image", nil
	case "video":
		return whatsmeow.MediaVideo, "video", nil
	case "audio":
		return whatsmeow.MediaAudio, "audio", nil
	case "document":
		return whatsmeow.MediaDocument, "document", nil
	}
	return "", "", errors.New("invalid media_type")
}

// net/http includes the request URL in transport errors. Upload URLs carry a
// session media authorization token, so only expose the operation's failure.
func nativeMediaError(operation string, err error) error {
	if errors.Is(err, context.Canceled) {
		return context.Canceled
	}
	if errors.Is(err, context.DeadlineExceeded) {
		return context.DeadlineExceeded
	}
	return fmt.Errorf("native media %s failed", operation)
}
func nativeMediaContent(args map[string]any, max int) ([]byte, []byte, error) {
	kind := stringArg(args, "media_type")
	if _, _, err := nativeMediaType(kind); err != nil {
		return nil, nil, err
	}
	if err := nativeText(stringArg(args, "caption"), "caption", 65536, true); err != nil {
		return nil, nil, err
	}
	if err := nativeText(stringArg(args, "file_name"), "file_name", 255, true); err != nil {
		return nil, nil, err
	}
	if value, exists := args["mimetype"]; exists && value != nil {
		mime := stringArg(args, "mimetype")
		if err := nativeText(mime, "mimetype", 200, false); err != nil {
			return nil, nil, err
		}
		if !strings.Contains(mime, "/") || strings.ContainsAny(mime, "\r\n") {
			return nil, nil, errors.New("invalid mimetype")
		}
		prefix := map[string]string{"image": "image/", "video": "video/", "audio": "audio/", "sticker": "image/webp"}[kind]
		if prefix != "" && !strings.HasPrefix(mime, prefix) {
			return nil, nil, errors.New("mimetype does not match media_type")
		}
	}
	if boolArg(args, "voice_note") && kind != "audio" {
		return nil, nil, errors.New("voice_note requires audio")
	}
	if boolArg(args, "gif_playback") && kind != "video" {
		return nil, nil, errors.New("gif_playback requires video")
	}
	if caption, exists := args["caption"]; exists && caption != nil && (kind == "audio" || kind == "sticker") {
		return nil, nil, errors.New("captions are supported only for images, videos and documents")
	}
	data, err := decodeMedia(stringArg(args, "data_base64"), max)
	if err != nil {
		return nil, nil, err
	}
	var thumb []byte
	if raw, ok := args["thumbnail_base64"]; ok && raw != nil {
		thumb, err = decodeMedia(stringArg(args, "thumbnail_base64"), min(max, 512*1024))
	}
	return data, thumb, err
}
func nativeMediaMessage(args map[string]any, upload whatsmeow.UploadResponse, thumb []byte) *waE2E.Message {
	kind := stringArg(args, "media_type")
	mime := stringArg(args, "mimetype")
	if mime == "" {
		mime = map[string]string{"image": "image/jpeg", "video": "video/mp4", "audio": "audio/ogg; codecs=opus", "document": "application/octet-stream", "sticker": "image/webp"}[kind]
	}
	caption, filename := stringArg(args, "caption"), stringArg(args, "file_name")
	seconds := uint32(intArg(args, "duration_seconds", 0))
	ts := time.Now().Unix()
	switch kind {
	case "image":
		return &waE2E.Message{ImageMessage: &waE2E.ImageMessage{URL: proto.String(upload.URL), DirectPath: proto.String(upload.DirectPath), MediaKey: upload.MediaKey, FileSHA256: upload.FileSHA256, FileEncSHA256: upload.FileEncSHA256, FileLength: proto.Uint64(upload.FileLength), MediaKeyTimestamp: &ts, Mimetype: &mime, Caption: &caption, JPEGThumbnail: thumb}}
	case "video":
		return &waE2E.Message{VideoMessage: &waE2E.VideoMessage{URL: proto.String(upload.URL), DirectPath: proto.String(upload.DirectPath), MediaKey: upload.MediaKey, FileSHA256: upload.FileSHA256, FileEncSHA256: upload.FileEncSHA256, FileLength: proto.Uint64(upload.FileLength), MediaKeyTimestamp: &ts, Mimetype: &mime, Caption: &caption, JPEGThumbnail: thumb, Seconds: &seconds, GifPlayback: proto.Bool(boolArg(args, "gif_playback"))}}
	case "audio":
		return &waE2E.Message{AudioMessage: &waE2E.AudioMessage{URL: proto.String(upload.URL), DirectPath: proto.String(upload.DirectPath), MediaKey: upload.MediaKey, FileSHA256: upload.FileSHA256, FileEncSHA256: upload.FileEncSHA256, FileLength: proto.Uint64(upload.FileLength), MediaKeyTimestamp: &ts, Mimetype: &mime, Seconds: &seconds, PTT: proto.Bool(boolArg(args, "voice_note"))}}
	case "document":
		return &waE2E.Message{DocumentMessage: &waE2E.DocumentMessage{URL: proto.String(upload.URL), DirectPath: proto.String(upload.DirectPath), MediaKey: upload.MediaKey, FileSHA256: upload.FileSHA256, FileEncSHA256: upload.FileEncSHA256, FileLength: proto.Uint64(upload.FileLength), MediaKeyTimestamp: &ts, Mimetype: &mime, Caption: &caption, FileName: &filename, JPEGThumbnail: thumb}}
	default:
		return &waE2E.Message{StickerMessage: &waE2E.StickerMessage{URL: proto.String(upload.URL), DirectPath: proto.String(upload.DirectPath), MediaKey: upload.MediaKey, FileSHA256: upload.FileSHA256, FileEncSHA256: upload.FileEncSHA256, FileLength: proto.Uint64(upload.FileLength), MediaKeyTimestamp: &ts, Mimetype: proto.String("image/webp")}}
	}
}
func nativeDownloadArgs(args map[string]any, max int) (string, []byte, []byte, []byte, int64, error) {
	path := stringArg(args, "direct_path")
	length := int64(nativeNumber(args, "file_length"))
	if len(path) > 8192 || (!strings.HasPrefix(path, "/mms/") && !strings.HasPrefix(path, "/v/")) || strings.ContainsAny(path, "\\#") || strings.IndexFunc(path, func(r rune) bool { return unicode.IsSpace(r) || unicode.IsControl(r) }) >= 0 {
		return "", nil, nil, nil, 0, errors.New("invalid media direct_path")
	}
	if length <= 0 || length > int64(max) {
		return "", nil, nil, nil, 0, errors.New("file_length must be positive and within max_media_bytes")
	}
	decode := func(key string, required bool) ([]byte, error) {
		value := stringArg(args, key)
		if value == "" && !required {
			return nil, nil
		}
		data, err := decodeMedia(value, 32)
		if err != nil || len(data) != 32 {
			return nil, fmt.Errorf("%s must encode exactly 32 bytes", key)
		}
		return data, nil
	}
	hash, err := decode("file_sha256_base64", true)
	if err != nil {
		return "", nil, nil, nil, 0, err
	}
	key, err := decode("media_key_base64", false)
	if err != nil {
		return "", nil, nil, nil, 0, err
	}
	enc, err := decode("file_enc_sha256_base64", false)
	if err != nil {
		return "", nil, nil, nil, 0, err
	}
	if (len(key) == 0) != (len(enc) == 0) {
		return "", nil, nil, nil, 0, errors.New("media_key_base64 and file_enc_sha256_base64 must be supplied together")
	}
	return path, hash, key, enc, length, nil
}
func registerNativeMedia(r *Registry) {
	r.Add("whatsapp::media::upload", func(ctx context.Context, args map[string]any) (any, error) {
		kind := stringArg(args, "media_type")
		mediaType, _, err := nativeMediaType(kind)
		if err != nil {
			return nil, err
		}
		config, configErr := r.Manager.ConfigContext(ctx)
		if configErr != nil {
			return nil, configErr
		}
		data, err := decodeMedia(stringArg(args, "data_base64"), config.MaxMediaBytes)
		if err != nil {
			return nil, err
		}
		client, err := r.Manager.ClientContext(ctx, stringArg(args, "session_id"))
		if err != nil {
			return nil, err
		}
		upload, err := client.Upload(ctx, data, mediaType)
		if err != nil {
			return nil, nativeMediaError("upload", err)
		}
		return map[string]any{"direct_path": upload.DirectPath, "url": upload.URL, "media_key_base64": base64.StdEncoding.EncodeToString(upload.MediaKey), "file_sha256_base64": base64.StdEncoding.EncodeToString(upload.FileSHA256), "file_enc_sha256_base64": base64.StdEncoding.EncodeToString(upload.FileEncSHA256), "file_length": upload.FileLength, "media_key_timestamp": time.Now().Unix(), "media_type": kind}, nil
	})
	r.Add("whatsapp::media::send", func(ctx context.Context, args map[string]any) (any, error) {
		chat, err := parseJID(stringArg(args, "chat_id"))
		if err != nil {
			return nil, err
		}
		if !nativeChat(chat) {
			return nil, errors.New("media sending requires a person or group")
		}
		media := nativeMap(args, "media")
		config, configErr := r.Manager.ConfigContext(ctx)
		if configErr != nil {
			return nil, configErr
		}
		data, thumb, err := nativeMediaContent(media, config.MaxMediaBytes)
		if err != nil {
			return nil, err
		}
		mediaType, _, _ := nativeMediaType(stringArg(media, "media_type"))
		client, err := r.Manager.ClientContext(ctx, stringArg(args, "session_id"))
		if err != nil {
			return nil, err
		}
		upload, err := client.Upload(ctx, data, mediaType)
		if err != nil {
			return nil, nativeMediaError("upload", err)
		}
		response, err := client.SendMessage(ctx, chat, nativeMediaMessage(media, upload, thumb))
		if err != nil {
			return nil, err
		}
		return nativeSent(response, chat), nil
	})
	r.Add("whatsapp::media::download", func(ctx context.Context, args map[string]any) (any, error) {
		mediaType, mmsType, err := nativeMediaType(stringArg(args, "media_type"))
		if err != nil {
			return nil, err
		}
		config, configErr := r.Manager.ConfigContext(ctx)
		if configErr != nil {
			return nil, configErr
		}
		path, hash, key, enc, length, err := nativeDownloadArgs(args, config.MaxMediaBytes)
		if err != nil {
			return nil, err
		}
		client, err := r.Manager.ClientContext(ctx, stringArg(args, "session_id"))
		if err != nil {
			return nil, err
		}
		limit := length
		if len(key) > 0 {
			limit += 26
		}
		ctx = context.WithValue(ctx, nativeMediaLimitKey{}, limit)
		data, err := client.DownloadMediaWithPath(ctx, path, enc, hash, key, mediaType, mmsType, false)
		if err != nil {
			return nil, nativeMediaError("download", err)
		}
		if int64(len(data)) != length {
			return nil, errors.New("downloaded media length does not match file_length")
		}
		return map[string]any{"data_base64": base64.StdEncoding.EncodeToString(data), "file_length": len(data)}, nil
	})
}
