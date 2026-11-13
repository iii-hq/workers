// Package client runs the mesh worker as a hub node inside a local project:
// it joins a remote hub without root and bridges the shared local workers'
// functions to the hub project as <node>::<fn>.
package client

import (
	"encoding/json"
	"errors"
	"fmt"
	"regexp"
	"slices"
	"strings"
)

// Config is the client section of the mesh configuration entry. The hub
// validates HubURL with its own URL rules before calling Validate.
type Config struct {
	HubURL string   `json:"hub_url,omitempty"`
	Join   string   `json:"join,omitempty"`
	Name   string   `json:"name,omitempty"`
	Share  []string `json:"share,omitempty"`
	// No comma in the description: the jsonschema tag splits on them.
	PeerToPeer bool `json:"peer_to_peer,omitempty" jsonschema:"description=Call machines that also turn this on directly and take their calls without the hub project in between."`
}

var namePattern = regexp.MustCompile(`^[a-z][a-z0-9-]{0,30}$`)
var workerPattern = regexp.MustCompile(`^[a-z0-9-]+$`)

// A function id shared alone: a prefix, "::" and the rest; no wildcard or space.
var functionPattern = regexp.MustCompile(`^[A-Za-z0-9_.-]+::[A-Za-z0-9_.:-]+$`)

// Bridged calls reach the engine as caller "mesh"; sharing these would hand
// other projects the mesh identity's secrets and configuration.
var deniedWorkers = map[string]bool{"mesh": true, "secrets": true, "configuration": true, "rbac-proxy": true}

func (c Config) Validate() error {
	if c.Join != "" && !strings.HasPrefix(c.Join, "secret://") && !strings.HasPrefix(c.Join, "env://") {
		return errors.New("join must be a secret:// or env:// reference, never a literal")
	}
	if c.Name != "" && !namePattern.MatchString(c.Name) {
		return fmt.Errorf("invalid name %q", c.Name)
	}
	return ValidateShare(c.Share)
}

// ValidateShare checks a share list: worker names, and function ids (with "::") shared alone.
func ValidateShare(share []string) error {
	for _, w := range share {
		switch {
		case !strings.Contains(w, "::"):
			if !workerPattern.MatchString(w) || deniedWorkers[w] {
				return fmt.Errorf("worker %q cannot be shared", w)
			}
		case len(w) > 256 || !functionPattern.MatchString(w):
			return fmt.Errorf("function %q cannot be shared: not a function id", w)
		// hub:: and peer:: are this machine's mirrors of other owners' functions.
		case !Bridgeable(w) || strings.HasPrefix(w, "hub::") || strings.HasPrefix(w, "peer::"):
			return fmt.Errorf("function %q cannot be shared: %s, hub:: and peer:: functions never leave a machine", w, strings.Join(deniedPrefixes, ", "))
		}
	}
	return nil
}

// ShareEntry returns the entry of share that shares a function: its id named
// alone, else its worker's name for a function that is not internal.
func ShareEntry(share []string, worker, id string, internal bool) string {
	// Whatever its id, a denied worker's function would hand out the mesh identity's secrets or configuration.
	if !deniedWorkers[worker] && slices.Contains(share, id) {
		return id
	}
	if !internal && slices.Contains(share, worker) {
		return worker
	}
	return ""
}

// SharesIDs says whether share names a function alone, which only a listing with internal rows and every worker finds.
func SharesIDs(share []string) bool {
	return slices.ContainsFunc(share, func(e string) bool { return strings.Contains(e, "::") })
}

// Internal says whether registration metadata marks a function internal, which engine::functions::list hides by default.
func Internal(metadata json.RawMessage) bool {
	var m struct {
		Internal bool `json:"internal"`
	}
	return json.Unmarshal(metadata, &m) == nil && m.Internal
}
