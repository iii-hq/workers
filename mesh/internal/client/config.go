// Package client runs the mesh worker as a hub node inside a local project:
// it joins a remote hub without root and bridges the shared local workers'
// functions to the hub project as <node>::<fn>.
package client

import (
	"errors"
	"fmt"
	"regexp"
	"strings"
)

// Config is the client section of the mesh configuration entry. The hub
// validates HubURL with its own URL rules before calling Validate.
type Config struct {
	HubURL string   `json:"hub_url,omitempty"`
	Join   string   `json:"join,omitempty"`
	Name   string   `json:"name,omitempty"`
	Share  []string `json:"share,omitempty"`
}

var namePattern = regexp.MustCompile(`^[a-z][a-z0-9-]{0,30}$`)
var workerPattern = regexp.MustCompile(`^[a-z0-9-]+$`)

// Bridged calls reach the local engine as caller "mesh"; sharing these would
// hand the hub project the mesh identity's secrets and configuration.
var deniedWorkers = map[string]bool{"mesh": true, "secrets": true, "configuration": true, "rbac-proxy": true}

func (c Config) Validate() error {
	if c.Join != "" && !strings.HasPrefix(c.Join, "secret://") && !strings.HasPrefix(c.Join, "env://") {
		return errors.New("join must be a secret:// or env:// reference, never a literal")
	}
	if c.Name != "" && !namePattern.MatchString(c.Name) {
		return fmt.Errorf("invalid name %q", c.Name)
	}
	for _, w := range c.Share {
		if !workerPattern.MatchString(w) || deniedWorkers[w] {
			return fmt.Errorf("worker %q cannot be shared", w)
		}
	}
	return nil
}
