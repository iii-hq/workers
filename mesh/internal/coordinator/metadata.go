package coordinator

import (
	"errors"
	"fmt"
	"strings"
	"unicode"
	"unicode/utf8"
)

type RuntimeMetadata struct {
	OS      string `json:"os"`
	Arch    string `json:"arch"`
	Version string `json:"version"`
}

// SetRuntimeMetadata accepts authenticated mesh-node status, never Tailscale
// hostinfo: its version identifies a different executable.
func (c *Coordinator) SetRuntimeMetadata(name string, metadata RuntimeMetadata) error {
	if metadata.OS == "" || metadata.Arch == "" || metadata.Version == "" {
		return errors.New("incomplete node runtime metadata")
	}
	for _, value := range []string{metadata.OS, metadata.Arch, metadata.Version} {
		if len(value) > 256 || !utf8.ValidString(value) || strings.TrimSpace(value) != value {
			return errors.New("invalid node runtime metadata")
		}
		for _, r := range value {
			if unicode.IsControl(r) {
				return errors.New("invalid node runtime metadata")
			}
		}
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	n := c.byName(name)
	if !c.authorized(n) {
		return fmt.Errorf("no authorized node %q", name)
	}
	old := n.RuntimeMetadata
	if old == metadata {
		return nil
	}
	n.RuntimeMetadata = metadata
	if err := c.save(); err != nil {
		n.RuntimeMetadata = old
		return err
	}
	if old.Version != metadata.Version {
		c.notifyNode(n, "version")
	}
	return nil
}
