package hub

import (
	"context"
	"encoding/json"
	"errors"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/assets"
)

type UIRequest struct {
	Path string `json:"path"`
}
type UIResponse struct {
	Content     string `json:"content"`
	ContentType string `json:"content_type"`
}

func uiContent(_ context.Context, request UIRequest) (UIResponse, error) {
	file, kind := "", ""
	switch request.Path {
	case "mesh/page.js":
		file, kind = "page.js", "text/javascript; charset=utf-8"
	case "mesh/styles.css":
		file, kind = "styles.css", "text/css; charset=utf-8"
	default:
		return UIResponse{}, errors.New("unknown mesh UI asset")
	}
	b, err := assets.Files.ReadFile(file)
	return UIResponse{Content: string(b), ContentType: kind}, err
}

func (h *Hub) registerUI(client *iii.Client) error {
	if _, err := iii.RegisterFunctionTyped(client, "mesh::ui-content", uiContent, iii.RegisterFunctionOptions{Metadata: json.RawMessage(`{"internal":true,"trace_hidden":true}`)}); err != nil {
		return err
	}
	for _, asset := range []struct{ path, kind string }{{"mesh/page.js", "script"}, {"mesh/styles.css", "style"}} {
		if _, err := client.Observe(iii.Source{FunctionID: "ade::ui::asset", Namespace: h.Namespace, Args: iii.Args{"path": asset.path, "kind": asset.kind}}, "mesh::ui-content", iii.ObserverOptions{ID: "mesh:ui:" + asset.path, TargetNamespace: h.Namespace}); err != nil {
			return err
		}
	}
	return nil
}
