package client

// Fields that only make sense on the machine a call came from. The machine that
// runs the call drops them, so it never trusts a stamp another machine wrote.

import (
	"encoding/json"
	"errors"
	"strings"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

// shortcut: a fixed list of the fields today's stamping hooks write (harness fs_scope, workflow::stamp-reply, a2ui::stamp-session, ade::working-directory::stamp-session), top level only; replace it with a shared marker for machine-bound fields when a new stamping hook appears.
func machineBound(id string) []string {
	keys := []string{"fs_scope", "caller_fs_root", "caller_session_id"}
	switch {
	case strings.HasPrefix(id, "a2ui::"):
		return append(keys, "session_id")
	case strings.HasPrefix(id, "ade::"):
		return append(keys, "session_id", "turn_id")
	}
	return keys
}

// DropMachineBound returns payload without the machine-bound top-level keys of id. A payload that holds none keeps its bytes.
func DropMachineBound(id string, payload json.RawMessage) json.RawMessage {
	var fields map[string]json.RawMessage
	if json.Unmarshal(payload, &fields) != nil {
		return payload
	}
	dropped := false
	for _, k := range machineBound(id) {
		if _, ok := fields[k]; ok {
			delete(fields, k)
			dropped = true
		}
	}
	if !dropped {
		return payload
	}
	out, _ := json.Marshal(fields) // cannot fail: every value came out of the decoder
	return out
}

// The marker approval-gate looks for, wherever it stands in the message.
const accessRequestMarker = "filesystem_access_request="

// DropAccessRequest cuts the folder request off an error of this machine.
// The tail asks the caller's approval gate for a folder of this machine. A grant would ride fs_scope, which this machine drops, so the request could never be met.
func DropAccessRequest(err error) error {
	var ie *iii.InvocationError
	if !errors.As(err, &ie) {
		return err
	}
	// The first marker, not the last: an error that wraps another one's tail would keep a request the gate can still parse.
	before, _, found := strings.Cut(ie.Message, accessRequestMarker)
	if !found {
		return err
	}
	c := *ie
	c.Message = strings.TrimSuffix(before, " ")
	return &c
}
