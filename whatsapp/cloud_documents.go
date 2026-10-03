package main

import (
	"archive/zip"
	"bytes"
	"context"
	"encoding/base64"
	"io"
	"mime/multipart"
	"net/http"
	"net/textproto"
	"path"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"
)

type CloudDocumentUploadRequest struct {
	DataBase64 string `json:"data_base64"`
	Mimetype   string `json:"mimetype"`
	FileName   string `json:"file_name"`
}

type CloudUploadedDocument struct {
	MediaID    string `json:"media_id"`
	Mimetype   string `json:"mimetype"`
	FileName   string `json:"file_name"`
	FileLength int    `json:"file_length"`
}

type CloudDocumentRequest struct {
	ChatID   string  `json:"chat_id"`
	MediaID  string  `json:"media_id"`
	FileName string  `json:"file_name"`
	Caption  *string `json:"caption"`
}

// Meta lists a 100 MB document limit. This conservative decimal ceiling also
// respects the worker's separately configured, usually smaller byte limit.
const cloudDocumentMaxBytes = 100_000_000

var cloudDocumentMIMEs = map[string]string{
	".txt":  "text/plain",
	".pdf":  "application/pdf",
	".xls":  "application/vnd.ms-excel",
	".xlsx": "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
	".doc":  "application/msword",
	".docx": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
	".ppt":  "application/vnd.ms-powerpoint",
	".pptx": "application/vnd.openxmlformats-officedocument.presentationml.presentation",
}

func cloudDocumentFilename(filename string) (string, error) {
	if strings.TrimSpace(filename) == "" || len(filename) > 255 || !utf8.ValidString(filename) ||
		strings.ContainsAny(filename, "/\\") || strings.IndexFunc(filename, unicode.IsControl) >= 0 || filename == "." || filename == ".." {
		return "", cloudError("invalid_request", "file_name must contain 1..255 bytes of valid UTF-8 without paths or control characters")
	}
	extension := strings.ToLower(path.Ext(filename))
	if extension == ".md" || extension == ".markdown" {
		return "", cloudError("cloud_unsupported_document", "Meta Cloud API does not officially support Markdown documents; explicitly choose a supported .txt or PDF file. The worker does not rename or convert files")
	}
	mimetype, ok := cloudDocumentMIMEs[extension]
	if !ok {
		return "", cloudError("cloud_unsupported_document", "file_name must use a supported document extension: .txt, .pdf, .xls, .xlsx, .doc, .docx, .ppt or .pptx")
	}
	return mimetype, nil
}

func cloudDocumentMediaID(id string) bool {
	if len(id) == 0 || len(id) > 128 {
		return false
	}
	for _, character := range id {
		if character < '0' || character > '9' {
			return false
		}
	}
	return true
}

func cloudDocumentContent(data []byte, filename, mimetype string) error {
	valid := false
	switch mimetype {
	case "text/plain":
		valid = utf8.Valid(data) && !bytes.ContainsRune(data, 0)
	case "application/pdf":
		valid = bytes.HasPrefix(data, []byte("%PDF-"))
	case "application/msword", "application/vnd.ms-excel", "application/vnd.ms-powerpoint":
		valid = bytes.HasPrefix(data, []byte{0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1})
	default:
		// Inspect ZIP metadata only: do not decompress untrusted Office files.
		reader, err := zip.NewReader(bytes.NewReader(data), int64(len(data)))
		if err == nil {
			entry := map[string]string{".docx": "word/document.xml", ".xlsx": "xl/workbook.xml", ".pptx": "ppt/presentation.xml"}[strings.ToLower(path.Ext(filename))]
			var hasTypes, hasDocument bool
			for _, file := range reader.File {
				hasTypes = hasTypes || file.Name == "[Content_Types].xml"
				hasDocument = hasDocument || file.Name == entry
			}
			valid = entry != "" && hasTypes && hasDocument
		}
	}
	if !valid {
		return cloudError("cloud_unsupported_document", "Document content does not match the declared MIME type; text/plain must be UTF-8 without NUL. The worker does not convert files")
	}
	return nil
}

func cloudDocumentPayload(req CloudDocumentRequest) (map[string]any, error) {
	if !cloudDocumentMediaID(req.MediaID) {
		return nil, cloudError("invalid_request", "media_id must contain 1..128 digits and refer to a document uploaded to this Cloud API account")
	}
	if _, err := cloudDocumentFilename(req.FileName); err != nil {
		return nil, err
	}
	if req.Caption != nil && (!utf8.ValidString(*req.Caption) || strings.ContainsRune(*req.Caption, 0) || utf8.RuneCountInString(*req.Caption) > 1024) {
		return nil, cloudError("invalid_request", "caption must be valid UTF-8, at most 1024 characters, without NUL")
	}
	body, err := cloudRecipient(req.ChatID)
	if err != nil {
		return nil, cloudError("invalid_request", "chat_id must be a Cloud API recipient; linked-device native JIDs are not supported")
	}
	document := map[string]any{"id": req.MediaID, "filename": req.FileName}
	if req.Caption != nil {
		document["caption"] = *req.Caption
	}
	body["type"] = "document"
	body["document"] = document
	return body, nil
}

func (c *cloudClient) uploadDocument(ctx context.Context, config Config, req CloudDocumentUploadRequest) (CloudUploadedDocument, error) {
	result := CloudUploadedDocument{}
	if err := cloudRequestConfig(config); err != nil {
		return result, err
	}
	mimetype, err := cloudDocumentFilename(req.FileName)
	if err != nil {
		return result, err
	}
	if req.Mimetype == "text/markdown" || req.Mimetype == "text/x-markdown" {
		return result, cloudError("cloud_unsupported_document", "Meta Cloud API does not officially support text/markdown; explicitly choose a supported .txt or PDF file. The worker does not rename or convert files")
	}
	if req.Mimetype != mimetype {
		return result, cloudError("cloud_unsupported_document", "mimetype must be a supported document MIME type matching file_name; MIME parameters are not supported")
	}
	limit := min(config.MaxMediaBytes, cloudDocumentMaxBytes)
	if limit < 1 || len(req.DataBase64) > base64.StdEncoding.EncodedLen(limit) {
		return result, cloudError("cloud_media_too_large", "Document exceeds max_media_bytes or the 100 MB Cloud API document ceiling")
	}
	// Strict base64 still permits CR/LF. Account for those and padding when
	// checking the final group before allocating the decoded document.
	encodedLength := len(req.DataBase64) - strings.Count(req.DataBase64, "\r") - strings.Count(req.DataBase64, "\n")
	encodedEnd := strings.TrimRight(req.DataBase64, "\r\n")
	decodedLength := base64.StdEncoding.DecodedLen(encodedLength)
	if strings.HasSuffix(encodedEnd, "=") {
		decodedLength--
	}
	if strings.HasSuffix(encodedEnd, "==") {
		decodedLength--
	}
	if decodedLength > limit {
		return result, cloudError("cloud_media_too_large", "Document exceeds max_media_bytes or the 100 MB Cloud API document ceiling")
	}
	data, err := decodeMedia(req.DataBase64, limit)
	if err != nil {
		return result, cloudError("invalid_request", "data_base64 must be strict base64 containing a nonempty document")
	}
	if err := cloudDocumentContent(data, req.FileName, mimetype); err != nil {
		return result, err
	}
	var envelope bytes.Buffer
	writer := multipart.NewWriter(&envelope)
	if err = writer.WriteField("messaging_product", "whatsapp"); err != nil {
		return result, cloudError("invalid_request", "Could not prepare document upload")
	}
	if err = writer.WriteField("type", mimetype); err != nil {
		return result, cloudError("invalid_request", "Could not prepare document upload")
	}
	header := textproto.MIMEHeader{}
	// RFC 7578 form-data uses filename, not filename*. Preserve UTF-8 as
	// multipart.Writer.CreateFormFile does; filename validation excludes slashes
	// and controls, leaving only quotes to escape in this header parameter.
	header.Set("Content-Disposition", `form-data; name="file"; filename="`+strings.ReplaceAll(req.FileName, `"`, `\"`)+`"`)
	header.Set("Content-Type", mimetype)
	if _, err = writer.CreatePart(header); err != nil {
		return result, cloudError("invalid_request", "Could not prepare document upload")
	}
	headerLength := envelope.Len()
	if err = writer.Close(); err != nil {
		return result, cloudError("invalid_request", "Could not prepare document upload")
	}
	// Keep only the envelope in the buffer; do not copy the document again.
	body := io.MultiReader(bytes.NewReader(envelope.Bytes()[:headerLength]), bytes.NewReader(data), bytes.NewReader(envelope.Bytes()[headerLength:]))
	response, err := c.requestBody(ctx, config, http.MethodPost, config.Cloud.PhoneNumberID+"/media", nil, body, writer.FormDataContentType())
	if err != nil {
		return result, err
	}
	id, _ := response["id"].(string)
	if !cloudDocumentMediaID(id) || cloudEchoesSecret(config, id) {
		return result, cloudError("cloud_outcome_unknown", "Cloud API returned no valid media ID; the upload may have completed. The worker does not retry automatically")
	}
	return CloudUploadedDocument{MediaID: id, Mimetype: mimetype, FileName: req.FileName, FileLength: len(data)}, nil
}

func registerCloudDocuments(r *Registry, client *cloudClient) {
	r.Add("whatsapp::cloud::media::upload_document", func(ctx context.Context, input map[string]any) (any, error) {
		var req CloudDocumentUploadRequest
		if err := decodeCloudRequest(input, &req); err != nil {
			return nil, cloudError("invalid_request", "Invalid document upload request")
		}
		return client.uploadDocument(ctx, r.Manager.Config(), req)
	})
	r.Add("whatsapp::cloud::messages::send_document", func(ctx context.Context, input map[string]any) (any, error) {
		var req CloudDocumentRequest
		if err := decodeCloudRequest(input, &req); err != nil {
			return nil, cloudError("invalid_request", "Invalid document message request")
		}
		body, err := cloudDocumentPayload(req)
		if err != nil {
			return nil, err
		}
		config := r.Manager.Config()
		if err := cloudRequestConfig(config); err != nil {
			return nil, err
		}
		inbox, err := r.Manager.CloudInbox(config)
		if err != nil {
			return nil, cloudError("cloud_storage_error", "Could not prepare Cloud API message archive; check the worker state and storage")
		}
		requestedAt := time.Now()
		sent, err := client.send(ctx, config, req.ChatID, body)
		if err != nil {
			return nil, err
		}
		text := req.FileName
		if req.Caption != nil && *req.Caption != "" {
			text = *req.Caption
		}
		if err = retainCloudSend(inbox, sent, "document", text, requestedAt); err != nil {
			return nil, cloudError("cloud_archive_failed", "Cloud API accepted document message "+sent.MessageID+", but local archival failed; do not resend automatically")
		}
		return sent, nil
	})
}
