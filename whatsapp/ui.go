package main

import (
	"context"
	"embed"
	"encoding/json"
	"errors"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

// Checked-in bundles allow `go build` without installing Node or pnpm.
// Rebuild them with `make ui` after editing the frontend sources.
//
//go:embed assets/page.js assets/styles.css
var uiAssets embed.FS

type UIRequest struct {
	Path string `json:"path"`
}
type UIResponse struct {
	Content     string `json:"content"`
	ContentType string `json:"content_type"`
}

func uiContent(_ context.Context, request UIRequest) (UIResponse, error) {
	file, contentType := "", ""
	switch request.Path {
	case "whatsapp/page.js":
		file = "assets/page.js"
		contentType = "text/javascript; charset=utf-8"
	case "whatsapp/styles.css":
		file = "assets/styles.css"
		contentType = "text/css; charset=utf-8"
	default:
		return UIResponse{}, errors.New("unknown WhatsApp UI asset")
	}
	b, err := uiAssets.ReadFile(file)
	return UIResponse{string(b), contentType}, err
}
func registerUI(client *iii.Client) error {
	if err := iii.RegisterFunctionTyped(client, "whatsapp::ui-content", uiContent, iii.RegisterFunctionOptions{Metadata: json.RawMessage(`{"internal":true}`)}); err != nil {
		return err
	}
	for _, asset := range []struct{ path, kind string }{{"whatsapp/page.js", "console:script"}, {"whatsapp/styles.css", "console:style"}} {
		config, _ := json.Marshal(map[string]string{"path": asset.path})
		if err := client.RegisterTriggerNamespaced("whatsapp:ui:"+asset.path, asset.kind, "whatsapp::ui-content", Namespace(), config); err != nil {
			return err
		}
	}
	return nil
}
