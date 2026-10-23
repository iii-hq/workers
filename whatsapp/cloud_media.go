package main

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/binary"
	"encoding/hex"
	"errors"
	"image"
	"image/color"
	_ "image/jpeg"
	_ "image/png"
	"io"
	"math"
	"mime"
	"mime/multipart"
	"net"
	"net/http"
	"net/netip"
	"net/textproto"
	"net/url"
	"path"
	"slices"
	"strconv"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"
)

type CloudMediaUploadRequest struct {
	MediaType  string `json:"media_type"`
	DataBase64 string `json:"data_base64"`
	Mimetype   string `json:"mimetype"`
	FileName   string `json:"file_name"`
}

type CloudUploadedMedia struct {
	MediaID    string `json:"media_id"`
	MediaType  string `json:"media_type"`
	Mimetype   string `json:"mimetype"`
	FileName   string `json:"file_name"`
	FileLength int    `json:"file_length"`
}

type CloudVoicemailUploadRequest struct {
	DataBase64  string `json:"data_base64"`
	Mimetype    string `json:"mimetype"`
	FileName    string `json:"file_name"`
	Description string `json:"description"`
}
type CloudUploadedVoicemail struct {
	CloudUploadedMedia
	UseCase string `json:"use_case"`
}

type CloudMediaIDRequest struct {
	MediaID string `json:"media_id"`
}

type CloudMediaInfo struct {
	MediaID    string `json:"media_id"`
	Mimetype   string `json:"mimetype"`
	FileLength int    `json:"file_length"`
	SHA256     string `json:"sha256"`
}

type CloudDownloadedMedia struct {
	CloudMediaInfo
	DataBase64 string `json:"data_base64"`
}

type CloudDeletedMedia struct {
	MediaID string `json:"media_id"`
	Deleted bool   `json:"deleted"`
}

type CloudMediaMessageRequest struct {
	ChatID                   string  `json:"chat_id"`
	MediaID                  string  `json:"media_id"`
	Caption                  *string `json:"caption"`
	ReplyToMessageID         string  `json:"reply_to_message_id"`
	RecipientIdentityKeyHash string  `json:"recipient_identity_key_hash"`
}

type cloudMediaSpec struct {
	kind, extension string
	limit           int
}

// Decimal ceilings are conservative interpretations of Meta's MB/KB limits.
var cloudMediaSpecs = map[string]cloudMediaSpec{
	"image/jpeg": {"image", ".jpeg", 5_000_000},
	"image/png":  {"image", ".png", 5_000_000},
	"audio/aac":  {"audio", ".aac", 16_000_000},
	"audio/amr":  {"audio", ".amr", 16_000_000},
	"audio/mpeg": {"audio", ".mp3", 16_000_000},
	"audio/mp4":  {"audio", ".m4a", 16_000_000},
	"audio/ogg":  {"audio", ".ogg", 16_000_000},
	"video/mp4":  {"video", ".mp4", 16_000_000},
	"video/3gpp": {"video", ".3gp", 16_000_000},
	"image/webp": {"sticker", ".webp", 500_000},
}

func cloudMediaMIME(value string) (string, cloudMediaSpec, error) {
	parsed, params, err := mime.ParseMediaType(value)
	if err != nil || len(params) > 0 && !(parsed == "audio/ogg" && len(params) == 1 && strings.EqualFold(params["codecs"], "opus")) {
		return "", cloudMediaSpec{}, cloudError("cloud_unsupported_media", "mimetype must be a supported Cloud API MIME type; only audio/ogg permits the codecs=opus parameter")
	}
	if spec, ok := cloudMediaSpecs[parsed]; ok {
		return parsed, spec, nil
	}
	for ext, documentMIME := range cloudDocumentMIMEs {
		if parsed == documentMIME {
			return parsed, cloudMediaSpec{"document", ext, cloudDocumentMaxBytes}, nil
		}
	}
	return "", cloudMediaSpec{}, cloudError("cloud_unsupported_media", "Unsupported Cloud API media MIME type; files are never renamed or converted")
}

func cloudMediaFilename(filename, mimetype string, spec cloudMediaSpec) error {
	if len(filename) == 0 || len(filename) > 255 || !utf8.ValidString(filename) || strings.ContainsAny(filename, "/\\") || strings.IndexFunc(filename, unicode.IsControl) >= 0 {
		return cloudError("invalid_request", "file_name must contain 1..255 bytes of valid UTF-8 without paths or control characters")
	}
	ext := strings.ToLower(path.Ext(filename))
	if ext != spec.extension && !(mimetype == "image/jpeg" && ext == ".jpg") {
		return cloudError("cloud_unsupported_media", "file_name extension must match the declared MIME type")
	}
	return nil
}

func cloudMediaContent(data []byte, filename, mimetype string) (int, error) {
	_, spec, err := cloudMediaMIME(mimetype)
	if err != nil {
		return 0, err
	}
	if spec.kind == "document" {
		return spec.limit, cloudDocumentContent(data, filename, mimetype)
	}
	valid := false
	limit := spec.limit
	switch mimetype {
	case "image/png", "image/jpeg":
		config, format, decodeErr := image.DecodeConfig(bytes.NewReader(data))
		valid = decodeErr == nil && config.Width > 0 && config.Height > 0 && "image/"+format == mimetype
		// Meta requires 8-bit RGB/RGBA; reject palette, grayscale and 16-bit PNG.
		if mimetype == "image/png" {
			valid = valid && len(data) > 25 && data[24] == 8 && (data[25] == 2 || data[25] == 6)
		} else {
			valid = valid && config.ColorModel == color.YCbCrModel
		}
	case "audio/aac":
		valid = len(data) > 7 && data[0] == 0xff && data[1]&0xf6 == 0xf0
	case "audio/amr":
		valid = bytes.HasPrefix(data, []byte("#!AMR\n")) && len(data) > 6
	case "audio/mpeg":
		frame := data
		if len(frame) >= 10 && string(frame[:3]) == "ID3" {
			size := 0
			for _, b := range frame[6:10] {
				if b >= 128 {
					size = len(data)
					break
				}
				size = size<<7 | int(b)
			}
			if size <= len(data)-10 {
				frame = data[10+size:]
			} else {
				frame = nil
			}
		}
		valid = len(frame) >= 4 && frame[0] == 0xff && frame[1]&0xe0 == 0xe0 && frame[1]&0x06 != 0 && frame[2]&0xf0 != 0xf0 && frame[2]&0x0c != 0x0c
	case "audio/ogg":
		// The first Ogg packet must be an Opus identification header with one channel.
		if len(data) >= 28 && string(data[:4]) == "OggS" && data[4] == 0 {
			headerEnd := 27 + int(data[26])
			valid = data[26] > 0 && headerEnd+19 <= len(data) && data[27] >= 19 && string(data[headerEnd:headerEnd+8]) == "OpusHead" && data[headerEnd+9] == 1
		}
	case "audio/mp4", "video/mp4", "video/3gpp":
		valid = cloudMediaMP4(data, spec.kind)
	case "image/webp":
		valid, limit = cloudMediaWebP(data)
	}
	if !valid {
		return 0, cloudError("cloud_unsupported_media", "Media content does not match a supported format or codec for the declared MIME type; the worker does not convert media")
	}
	return limit, nil
}

// Inspect bounded ISO BMFF metadata, without decoding media or spawning codecs.
func cloudMediaMP4(data []byte, kind string) bool {
	if len(data) < 16 || string(data[4:8]) != "ftyp" {
		return false
	}
	var audio, video int
	var walk func([]byte, int) bool
	walk = func(boxes []byte, depth int) bool {
		if depth > 8 {
			return false
		}
		for len(boxes) > 0 {
			if len(boxes) < 8 {
				return false
			}
			size, header := uint64(binary.BigEndian.Uint32(boxes[:4])), 8
			if size == 1 {
				if len(boxes) < 16 {
					return false
				}
				size = binary.BigEndian.Uint64(boxes[8:16])
				header = 16
			}
			if size == 0 {
				size = uint64(len(boxes))
			}
			if size < uint64(header) || size > uint64(len(boxes)) {
				return false
			}
			body := boxes[header:int(size)]
			switch string(boxes[4:8]) {
			case "moov", "trak", "mdia", "minf", "stbl":
				if !walk(body, depth+1) {
					return false
				}
			case "stsd":
				if len(body) < 8 || binary.BigEndian.Uint32(body[4:8]) != 1 || len(body[8:]) < 8 {
					return false
				}
				entry := body[8:]
				if int(binary.BigEndian.Uint32(entry[:4])) != len(entry) {
					return false
				}
				switch string(entry[4:8]) {
				case "mp4a":
					if len(entry) < 36 {
						return false
					}
					audio++
				case "avc1", "avc3":
					if len(entry) < 86 {
						return false
					}
					video++
				default:
					return false
				}
			}
			boxes = boxes[int(size):]
		}
		return true
	}
	if !walk(data, 0) {
		return false
	}
	return kind == "audio" && audio == 1 && video == 0 || kind == "video" && video == 1 && audio <= 1
}

func cloudMediaWebP(data []byte) (bool, int) {
	if len(data) < 20 || string(data[:4]) != "RIFF" || string(data[8:12]) != "WEBP" || uint64(binary.LittleEndian.Uint32(data[4:8]))+8 != uint64(len(data)) {
		return false, 0
	}
	var imageChunk, animationHeader, animationFrame, animationFlag bool
	for chunks := data[12:]; len(chunks) > 0; {
		if len(chunks) < 8 {
			return false, 0
		}
		size := uint64(binary.LittleEndian.Uint32(chunks[4:8]))
		padded := size + (size & 1)
		if padded > uint64(len(chunks)-8) {
			return false, 0
		}
		body := chunks[8 : 8+int(size)]
		switch string(chunks[:4]) {
		case "VP8X":
			if len(body) != 10 {
				return false, 0
			}
			animationFlag = body[0]&2 != 0
		case "VP8 ":
			imageChunk = len(body) >= 10 && bytes.Equal(body[3:6], []byte{0x9d, 0x01, 0x2a})
		case "VP8L":
			imageChunk = len(body) >= 5 && body[0] == 0x2f
		case "ANIM":
			animationHeader = len(body) == 6
		case "ANMF":
			animationFrame = len(body) > 16
		}
		chunks = chunks[8+int(padded):]
	}
	if animationFlag {
		return animationHeader && animationFrame, 500_000
	}
	return imageChunk && !animationHeader && !animationFrame, 100_000
}

func (c *cloudClient) uploadMedia(ctx context.Context, config Config, req CloudMediaUploadRequest, fields ...string) (CloudUploadedMedia, error) {
	var result CloudUploadedMedia
	if req.MediaType == "document" {
		document, err := c.uploadDocument(ctx, config, CloudDocumentUploadRequest{DataBase64: req.DataBase64, Mimetype: req.Mimetype, FileName: req.FileName})
		if err != nil {
			return result, err
		}
		return CloudUploadedMedia{document.MediaID, "document", document.Mimetype, document.FileName, document.FileLength}, nil
	}
	if err := cloudRequestConfig(config); err != nil {
		return result, err
	}
	mimetype, spec, err := cloudMediaMIME(req.Mimetype)
	if err != nil {
		return result, err
	}
	if req.MediaType != spec.kind || spec.kind == "document" {
		return result, cloudError("cloud_unsupported_media", "media_type must match the supported MIME type")
	}
	if err := cloudMediaFilename(req.FileName, mimetype, spec); err != nil {
		return result, err
	}
	limit := min(config.MaxMediaBytes, spec.limit)
	if limit < 1 || len(req.DataBase64) > base64.StdEncoding.EncodedLen(limit) {
		return result, cloudError("cloud_media_too_large", "Media exceeds max_media_bytes or the Meta media-type limit")
	}
	data, err := decodeMedia(req.DataBase64, limit)
	if err != nil {
		if len(req.DataBase64)/4*3-strings.Count(strings.TrimRight(req.DataBase64, "\r\n"), "=") > limit {
			return result, cloudError("cloud_media_too_large", "Media exceeds max_media_bytes or the Meta media-type limit")
		}
		return result, cloudError("invalid_request", "data_base64 must contain nonempty strict base64 within the configured byte limit")
	}
	contentLimit, err := cloudMediaContent(data, req.FileName, mimetype)
	if err != nil {
		return result, err
	}
	if len(data) > contentLimit {
		return result, cloudError("cloud_media_too_large", "Media exceeds the Meta media-type limit; static stickers are limited to 100 KB")
	}
	id, err := c.postCloudMedia(ctx, config, req.FileName, mimetype, data, fields...)
	if err != nil {
		return result, err
	}
	return CloudUploadedMedia{id, req.MediaType, mimetype, req.FileName, len(data)}, nil
}

// cloudMultipartFile builds fields (name, value pairs) plus one file part and
// streams data between the envelope halves instead of copying it, so the
// request is sent chunked; callers that need Content-Length buffer it. Writes
// to a bytes.Buffer cannot fail.
func cloudMultipartFile(fileName, mimetype string, data []byte, fields ...string) (io.Reader, string) {
	var envelope bytes.Buffer
	writer := multipart.NewWriter(&envelope)
	for i := 0; i+1 < len(fields); i += 2 {
		_ = writer.WriteField(fields[i], fields[i+1])
	}
	header := textproto.MIMEHeader{}
	// RFC 7578 form-data uses filename, not filename*. Preserve UTF-8 as
	// multipart.Writer.CreateFormFile does; callers' filename validation excludes
	// slashes and controls, leaving only quotes to escape in this header parameter.
	header.Set("Content-Disposition", `form-data; name="file"; filename="`+strings.ReplaceAll(fileName, `"`, `\"`)+`"`)
	header.Set("Content-Type", mimetype)
	_, _ = writer.CreatePart(header)
	headerLength := envelope.Len()
	_ = writer.Close()
	return io.MultiReader(bytes.NewReader(envelope.Bytes()[:headerLength]), bytes.NewReader(data), bytes.NewReader(envelope.Bytes()[headerLength:])), writer.FormDataContentType()
}

// postCloudMedia uploads one file to the phone number's media store and returns
// the media ID Meta assigned.
func (c *cloudClient) postCloudMedia(ctx context.Context, config Config, fileName, mimetype string, data []byte, fields ...string) (string, error) {
	body, contentType := cloudMultipartFile(fileName, mimetype, data, append([]string{"messaging_product", "whatsapp", "type", mimetype}, fields...)...)
	response, err := c.requestBody(ctx, config, http.MethodPost, config.Cloud.PhoneNumberID+"/media", nil, body, contentType)
	if err != nil {
		return "", err
	}
	id, _ := response["id"].(string)
	if !cloudDocumentMediaID(id) || cloudEchoesSecret(config, id) {
		return "", cloudError("cloud_outcome_unknown", "Cloud API returned no valid media ID; the upload may have completed. No automatic retry was made")
	}
	return id, nil
}

// Ogg granule positions for Opus are measured at 48 kHz. Require a complete,
// single logical stream and use its final granule minus pre-skip for duration.
func cloudVoicemailDuration(data []byte) bool {
	if len(data) < 47 || string(data[:4]) != "OggS" {
		return false
	}
	headerEnd := 27 + int(data[26])
	if headerEnd+19 > len(data) || string(data[headerEnd:headerEnd+8]) != "OpusHead" {
		return false
	}
	preSkip := uint64(binary.LittleEndian.Uint16(data[headerEnd+10 : headerEnd+12]))
	serial := binary.LittleEndian.Uint32(data[14:18])
	sequence := uint32(0)
	ended := false
	var granule uint64
	for len(data) > 0 {
		if ended || len(data) < 27 || string(data[:4]) != "OggS" || data[4] != 0 || binary.LittleEndian.Uint32(data[14:18]) != serial || binary.LittleEndian.Uint32(data[18:22]) != sequence {
			return false
		}
		segments := int(data[26])
		if len(data) < 27+segments {
			return false
		}
		size := 27 + segments
		for _, length := range data[27 : 27+segments] {
			size += int(length)
		}
		if size > len(data) {
			return false
		}
		granule = binary.LittleEndian.Uint64(data[6:14])
		ended = data[5]&4 != 0
		sequence++
		data = data[size:]
	}
	return ended && granule > preSkip && granule-preSkip < 60*48000
}

func (c *cloudClient) uploadVoicemail(ctx context.Context, config Config, req CloudVoicemailUploadRequest) (CloudUploadedVoicemail, error) {
	var result CloudUploadedVoicemail
	if cloudFieldText("description", req.Description, 1024, false) != nil {
		return result, cloudError("invalid_request", "description must contain at most 1024 characters")
	}
	// uploadMedia repeats every other check; only the duration is voicemail-specific.
	if data, err := decodeMedia(req.DataBase64, min(config.MaxMediaBytes, 16_000_000)); err == nil && !cloudVoicemailDuration(data) {
		return result, cloudError("cloud_unsupported_media", "Voicemail announcements require a complete mono Opus stream shorter than 60 seconds")
	}
	fields := []string{"use_case", "call_voicemail_announcement"}
	if req.Description != "" {
		fields = append(fields, "description", req.Description)
	}
	uploaded, err := c.uploadMedia(ctx, config, CloudMediaUploadRequest{MediaType: "audio", DataBase64: req.DataBase64, Mimetype: req.Mimetype, FileName: req.FileName}, fields...)
	if err != nil {
		return result, err
	}
	return CloudUploadedVoicemail{uploaded, "call_voicemail_announcement"}, nil
}

func (c *cloudClient) mediaMetadata(ctx context.Context, config Config, id string) (CloudMediaInfo, string, error) {
	var result CloudMediaInfo
	if !cloudDocumentMediaID(id) {
		return result, "", cloudError("invalid_request", "media_id must contain 1..128 digits")
	}
	response, err := c.request(ctx, config, http.MethodGet, id, url.Values{"phone_number_id": {config.Cloud.PhoneNumberID}}, nil)
	if err != nil {
		return result, "", err
	}
	returnedID, _ := response["id"].(string)
	mimetype, _ := response["mime_type"].(string)
	digest, _ := response["sha256"].(string)
	mediaURL, _ := response["url"].(string)
	length := -1
	switch size := response["file_size"].(type) {
	case string:
		if len(size) < 20 {
			parsed, e := strconv.ParseInt(size, 10, 64)
			if e == nil && parsed >= 0 && parsed <= math.MaxInt {
				length = int(parsed)
			}
		}
	case float64:
		if size >= 0 && size < math.MaxInt && math.Trunc(size) == size {
			length = int(size)
		}
	}
	hash, hashErr := hex.DecodeString(digest)
	if hashErr != nil || len(hash) != sha256.Size {
		hash, hashErr = base64.StdEncoding.Strict().DecodeString(digest)
	}
	if returnedID != id || cloudSanitizeResult(config, returnedID) != returnedID || cloudSanitizeResult(config, digest) != digest || cloudSanitizeResult(config, string(hash)) != string(hash) || length <= 0 || hashErr != nil || len(hash) != sha256.Size {
		return result, "", cloudError("cloud_invalid_response", "Cloud API returned invalid media metadata")
	}
	canonical, _, err := cloudMediaMIME(mimetype)
	if err != nil {
		return result, "", err
	}
	return CloudMediaInfo{id, canonical, length, hex.EncodeToString(hash)}, mediaURL, nil
}

func (c *cloudClient) getMedia(ctx context.Context, config Config, id string) (CloudMediaInfo, error) {
	result, _, err := c.mediaMetadata(ctx, config, id)
	return result, err
}

func (c *cloudClient) deleteMedia(ctx context.Context, config Config, id string) (CloudDeletedMedia, error) {
	var result CloudDeletedMedia
	if !cloudDocumentMediaID(id) {
		return result, cloudError("invalid_request", "media_id must contain 1..128 digits")
	}
	response, err := c.request(ctx, config, http.MethodDelete, id, url.Values{"phone_number_id": {config.Cloud.PhoneNumberID}}, nil)
	if err != nil {
		return result, err
	}
	if response["success"] != true {
		return result, cloudError("cloud_invalid_response", "Cloud API did not confirm media deletion; no automatic retry was made")
	}
	return CloudDeletedMedia{id, true}, nil
}

func cloudMediaDownloadURL(raw string) (*url.URL, error) {
	u, err := url.Parse(raw)
	if err != nil || len(raw) > 16384 || u.Scheme != "https" || u.User != nil || u.Fragment != "" || u.Opaque != "" || (u.Port() != "" && u.Port() != "443") {
		return nil, cloudError("cloud_unsafe_media_url", "Cloud API returned an unsafe media download URL")
	}
	host := strings.ToLower(u.Hostname())
	if host != "lookaside.fbsbx.com" && host != "mmg.whatsapp.net" && !strings.HasSuffix(host, ".fbcdn.net") {
		return nil, cloudError("cloud_unsafe_media_url", "Cloud API returned an unsupported media download host")
	}
	return u, nil
}

var cloudReservedMediaRanges = []netip.Prefix{
	netip.MustParsePrefix("0.0.0.0/8"), netip.MustParsePrefix("100.64.0.0/10"), netip.MustParsePrefix("192.0.0.0/24"), netip.MustParsePrefix("192.0.2.0/24"), netip.MustParsePrefix("192.88.99.0/24"), netip.MustParsePrefix("198.18.0.0/15"), netip.MustParsePrefix("198.51.100.0/24"), netip.MustParsePrefix("203.0.113.0/24"), netip.MustParsePrefix("240.0.0.0/4"), netip.MustParsePrefix("2001::/23"), netip.MustParsePrefix("2001:db8::/32"), netip.MustParsePrefix("2002::/16"), netip.MustParsePrefix("64:ff9b::/96"), netip.MustParsePrefix("64:ff9b:1::/48"), netip.MustParsePrefix("100::/64"),
}

func cloudPublicMediaIP(ip net.IP) bool {
	if !ip.IsGlobalUnicast() || ip.IsPrivate() || ip.IsLoopback() || ip.IsLinkLocalUnicast() {
		return false
	}
	address, ok := netip.AddrFromSlice(ip)
	if !ok {
		return false
	}
	address = address.Unmap()
	for _, reserved := range cloudReservedMediaRanges {
		if reserved.Contains(address) {
			return false
		}
	}
	return true
}

func cloudMediaDial(ctx context.Context, network, address string) (net.Conn, error) {
	host, port, err := net.SplitHostPort(address)
	if err != nil {
		return nil, errors.New("invalid media address")
	}
	addresses, err := net.DefaultResolver.LookupIPAddr(ctx, host)
	if err != nil || len(addresses) == 0 {
		return nil, errors.New("could not resolve media host")
	}
	for _, resolved := range addresses {
		if !cloudPublicMediaIP(resolved.IP) {
			return nil, errors.New("media host resolved to a restricted address")
		}
	}
	// Connect to the checked IP itself to avoid a second lookup/rebinding.
	return (&net.Dialer{Timeout: 15 * time.Second}).DialContext(ctx, network, net.JoinHostPort(addresses[0].IP.String(), port))
}

func (c *cloudClient) downloadMedia(ctx context.Context, config Config, id string) (CloudDownloadedMedia, error) {
	var result CloudDownloadedMedia
	ctx, cancel := context.WithTimeout(ctx, time.Duration(config.OperationTimeoutSecs)*time.Second)
	defer cancel()
	metadata, rawURL, err := c.mediaMetadata(ctx, config, id)
	if err != nil {
		return result, err
	}
	u, err := cloudMediaDownloadURL(rawURL)
	if err != nil {
		return result, err
	}
	_, spec, err := cloudMediaMIME(metadata.Mimetype)
	if err != nil {
		return result, err
	}
	limit := min(config.MaxMediaBytes, spec.limit)
	if limit < 1 || metadata.FileLength > limit {
		return result, cloudError("cloud_media_too_large", "Media exceeds max_media_bytes or the Meta media-type limit")
	}
	data, err := c.guardedDownload(ctx, config, u, limit, []string{metadata.Mimetype}, cloudError("cloud_invalid_response", "Media download was incomplete; no automatic retry was made"))
	if err != nil {
		return result, err
	}
	if len(data) != metadata.FileLength {
		return result, cloudError("cloud_invalid_response", "Media download did not match its declared size; no automatic retry was made")
	}
	digest := sha256.Sum256(data)
	if hex.EncodeToString(digest[:]) != metadata.SHA256 {
		return result, cloudError("cloud_invalid_response", "Media download did not match its declared SHA-256 digest")
	}
	contentLimit, err := cloudMediaContent(data, "download"+spec.extension, metadata.Mimetype)
	if err != nil {
		return result, err
	}
	if len(data) > contentLimit {
		return result, cloudError("cloud_media_too_large", "Downloaded media exceeds the Meta media-type limit")
	}
	return CloudDownloadedMedia{metadata, base64.StdEncoding.EncodeToString(data)}, nil
}

// guardedDownload GETs a URL already checked by cloudMediaDownloadURL. It never
// follows redirects, bypasses proxies, re-checks every resolved address,
// rejects a Content-Type outside mimetypes before reading the body and reads at
// most limit bytes; a failed body read returns incomplete.
func (c *cloudClient) guardedDownload(ctx context.Context, config Config, u *url.URL, limit int, mimetypes []string, incomplete error) ([]byte, error) {
	ctx, cancel := context.WithTimeout(ctx, time.Duration(config.OperationTimeoutSecs)*time.Second)
	defer cancel()
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, u.String(), nil)
	if err != nil {
		return nil, cloudError("cloud_unsafe_media_url", "Cloud API returned an invalid download URL")
	}
	request.Header.Set("Authorization", "Bearer "+config.Cloud.AccessToken)
	request.Header.Set("User-Agent", "iii-whatsapp-cloud/0.1")
	client := *c.http
	client.CheckRedirect = func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }
	if transport, ok := client.Transport.(*http.Transport); ok {
		guarded := transport.Clone()
		guarded.Proxy = nil
		guarded.DialContext = cloudMediaDial
		guarded.DialTLSContext = nil
		guarded.DialTLS = nil
		guarded.DisableKeepAlives = true
		client.Transport = guarded
		defer guarded.CloseIdleConnections()
	}
	response, err := client.Do(request)
	if err != nil {
		return nil, cloudError("cloud_download_error", "Cloud API download failed or timed out; no automatic retry was made")
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusOK {
		return nil, cloudError("cloud_download_error", "Cloud API download was not successful; the URL may have expired. No redirect or automatic retry was made")
	}
	if limit < 1 || response.ContentLength > int64(limit) {
		return nil, cloudError("cloud_media_too_large", "Download exceeds the configured byte limit")
	}
	if mimetype, _, _ := mime.ParseMediaType(response.Header.Get("Content-Type")); !slices.Contains(mimetypes, mimetype) {
		return nil, cloudError("cloud_unsupported_media", "Downloaded content type does not match the expected media type")
	}
	data, err := io.ReadAll(io.LimitReader(response.Body, int64(limit)+1))
	if len(data) > limit {
		return nil, cloudError("cloud_media_too_large", "Download exceeds the configured byte limit")
	}
	if err != nil {
		return nil, incomplete
	}
	return data, nil
}

func cloudMediaMessagePayload(kind string, req CloudMediaMessageRequest) (map[string]any, error) {
	if !cloudDocumentMediaID(req.MediaID) {
		return nil, cloudError("invalid_request", "media_id must contain 1..128 digits and refer to media uploaded to this Cloud API account")
	}
	if req.Caption != nil && (kind == "audio" || kind == "sticker") {
		return nil, cloudError("invalid_request", "Captions are supported only for image, video and document messages")
	}
	if req.Caption != nil && (!utf8.ValidString(*req.Caption) || strings.ContainsRune(*req.Caption, 0) || utf8.RuneCountInString(*req.Caption) > 1024) {
		return nil, cloudError("invalid_request", "caption must be valid UTF-8 and at most 1024 characters without NUL")
	}
	body, err := cloudRecipient(req.ChatID)
	if err != nil {
		return nil, cloudError("invalid_request", "chat_id must be a Cloud API recipient; native JIDs are not supported")
	}
	media := map[string]any{"id": req.MediaID}
	if req.Caption != nil {
		media["caption"] = *req.Caption
	}
	body["type"], body[kind] = kind, media
	if err := cloudApplyReply(body, req.ReplyToMessageID); err != nil {
		return nil, err
	}
	if err := cloudApplyIdentity(body, req.RecipientIdentityKeyHash); err != nil {
		return nil, err
	}
	return body, nil
}

func registerCloudMedia(r *Registry, client *cloudClient) {
	r.Add("whatsapp::cloud::media::upload_voicemail_announcement", func(ctx context.Context, input map[string]any) (any, error) {
		var req CloudVoicemailUploadRequest
		if decodeCloudRequest(input, &req) != nil {
			return nil, cloudError("invalid_request", "Invalid voicemail upload request")
		}
		config, configErr := r.Manager.ConfigContext(ctx)
		if configErr != nil {
			return nil, configErr
		}
		return client.uploadVoicemail(ctx, config, req)
	})
	r.Add("whatsapp::cloud::media::upload", func(ctx context.Context, input map[string]any) (any, error) {
		var req CloudMediaUploadRequest
		if decodeCloudRequest(input, &req) != nil {
			return nil, cloudError("invalid_request", "Invalid media upload request")
		}
		config, configErr := r.Manager.ConfigContext(ctx)
		if configErr != nil {
			return nil, configErr
		}
		return client.uploadMedia(ctx, config, req)
	})
	for _, operation := range []string{"get", "download", "delete"} {
		r.Add("whatsapp::cloud::media::"+operation, func(ctx context.Context, input map[string]any) (any, error) {
			var req CloudMediaIDRequest
			if decodeCloudRequest(input, &req) != nil {
				return nil, cloudError("invalid_request", "Invalid media ID request")
			}
			config, err := r.Manager.ConfigContext(ctx)
			if err != nil {
				return nil, err
			}
			switch operation {
			case "get":
				return client.getMedia(ctx, config, req.MediaID)
			case "download":
				return client.downloadMedia(ctx, config, req.MediaID)
			default:
				return client.deleteMedia(ctx, config, req.MediaID)
			}
		})
	}
	for _, kind := range []string{"image", "audio", "video", "sticker"} {
		r.Add("whatsapp::cloud::messages::send_"+kind, func(ctx context.Context, input map[string]any) (any, error) {
			var req CloudMediaMessageRequest
			if decodeCloudRequest(input, &req) != nil {
				return nil, cloudError("invalid_request", "Invalid media message request")
			}
			body, err := cloudMediaMessagePayload(kind, req)
			if err != nil {
				return nil, err
			}
			preview := strings.ToUpper(kind[:1]) + kind[1:]
			if req.Caption != nil && *req.Caption != "" {
				preview = *req.Caption
			}
			return cloudSendAndRetain(ctx, r, client, req.ChatID, body, kind, preview)
		})
	}
}
