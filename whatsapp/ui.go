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
	if _, err := iii.RegisterFunctionTyped(client, "whatsapp::ui-content", uiContent, iii.RegisterFunctionOptions{Metadata: json.RawMessage(`{"internal":true}`)}); err != nil {
		return err
	}
	// The console accepts each asset's observer and calls whatsapp::ui-content
	// through it with {path}.
	for _, asset := range []struct{ path, kind string }{{"whatsapp/page.js", "script"}, {"whatsapp/styles.css", "style"}} {
		if err := observeFunction(client, iii.Source{
			FunctionID: "console::ui::asset",
			Args:       iii.Args{"kind": asset.kind, "path": asset.path},
		}, "whatsapp::ui-content"); err != nil {
			return err
		}
	}
	return nil
}
