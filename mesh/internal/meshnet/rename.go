package meshnet

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"

	"tailscale.com/types/key"
)

// renamePath is the hub's rename endpoint on the control channel (internal/coordinator).
const renamePath = "/machine/rename"

// ErrRenameUnsupported is a hub built before machines could be renamed.
var ErrRenameUnsupported = errors.New("the hub cannot rename machines; update the hub's mesh worker")

// RenameRefused is the hub's own answer; any other error means the hub was not reached.
type RenameRefused struct{ Reason string }

func (e *RenameRefused) Error() string { return e.Reason }

// Rename returns the name the hub gave, which may differ from name; SelfName follows with the hub's next map.
func (m *Mesh) Rename(ctx context.Context, name string) (string, error) {
	body, err := json.Marshal(struct {
		NodeKey key.NodePublic `json:"node_key"`
		Name    string         `json:"name"`
	}{m.lb.NodeKey(), name})
	if err != nil {
		return "", err
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, "https://unused"+renamePath, bytes.NewReader(body))
	if err != nil {
		return "", err
	}
	res, err := m.lb.DoNoiseRequest(req)
	if err != nil {
		return "", err
	}
	defer res.Body.Close()
	return renameAnswer(res.StatusCode, io.LimitReader(res.Body, 1<<16))
}

func renameAnswer(status int, body io.Reader) (string, error) {
	if status == http.StatusNotFound || status == http.StatusMethodNotAllowed {
		return "", ErrRenameUnsupported
	}
	raw, _ := io.ReadAll(body)
	var out struct{ Name, Error string }
	if status != http.StatusOK || json.Unmarshal(raw, &out) != nil {
		return "", fmt.Errorf("hub answered %d to the rename: %s", status, bytes.TrimSpace(raw))
	}
	if out.Error != "" {
		return "", &RenameRefused{out.Error}
	}
	return out.Name, nil
}
