package hub

import (
	"cmp"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"log"
	"maps"
	"reflect"
	"slices"

	meshclient "github.com/iii-hq/workers/mesh/internal/client"
	"github.com/iii-hq/workers/mesh/internal/node"
)

// shortcut: fixed caps on what one owner and the whole catalog may share; make them settings when a real mesh reaches them.
const (
	maxOwnerFunctions = 500
	maxOwnerBytes     = 1 << 20
	maxCatalogBytes   = 8 << 20
)

// shared is what the last catalog read found.
type shared struct {
	catalog node.Catalog
	rows    map[rowKey]sharedRow               // catalog entries by owner and function id
	workers map[string]meshclient.SharedStatus // the share entries read, by entry
	skipped []string                           // machines whose functions do not fit
}
type rowKey struct{ owner, id string }
type sharedRow struct {
	owner, worker string
	internal      bool
}

func bridgeWorker(machine string) string { return "rbac-proxy:" + machine + "::mesh-bridge" }

// refreshShared reads the catalog with no lock held and swaps it in: the hub's
// shared functions and those of every machine on the hub's version.
func (h *Hub) refreshShared(ctx context.Context) error {
	engine := h.engineRPC()
	if engine == nil || h.Coordinator == nil {
		return errors.New("hub engine is not connected")
	}
	share := h.Config().Share
	ids := meshclient.SharesIDs(share)
	found := map[string]map[string]meshclient.Function{} // by owner, then function id
	entryOf := map[string]string{}                       // the share entry of each hub row kept, by id
	// One row past the cap still skips the owner below, and no schema past it is read.
	capped := func(keep func(meshclient.Row) bool) func(meshclient.Row) bool {
		kept := 0
		return func(r meshclient.Row) bool {
			if !keep(r) {
				return false
			}
			kept++
			return kept <= maxOwnerFunctions+1
		}
	}
	if len(share) > 0 {
		workers := slices.Clone(share)
		if ids {
			workers = nil // an id names no worker: list them all
		}
		// Internal rows too: an id entry may cite one.
		rows, err := meshclient.Discover(ctx, engine, h.Namespace, workers, true, capped(func(r meshclient.Row) bool {
			entry := meshclient.ShareEntry(share, r.WorkerName, r.FunctionID, meshclient.Internal(r.Metadata))
			if r.Namespace != h.Namespace || entry == "" || !meshclient.Bridgeable(r.FunctionID) {
				return false
			}
			entryOf[r.FunctionID] = entry
			return true
		}), nil)
		if err != nil {
			return err
		}
		found["hub"] = rows
	}
	// shortcut: one functions::list per machine on the hub's version; fold them into one listing keyed by (namespace, id) when a mesh has enough machines to feel it.
	for _, n := range h.Coordinator.Nodes() {
		if n.Name == "hub" || n.Name == h.Namespace || n.Version != h.Version {
			continue
		}
		bridge := bridgeWorker(n.Name)
		// Internal rows too: a machine's are the functions it shares by id.
		rows, err := meshclient.Discover(ctx, engine, n.Name, []string{bridge}, true, capped(func(r meshclient.Row) bool {
			// shortcut: a machine's rows carry the bridge's worker name only once its SDK has declared itself; a pass that lands before that (a slow link: rbac-proxy fixes the namespace first, so registrations apply as they arrive) leaves the machine out until its next function change. Observe engine::on-workers-changed and run the pass again when a machine is seen missing in practice.
			return r.Namespace == n.Name && r.WorkerName == bridge && meshclient.Bridgeable(r.FunctionID)
		}), nil)
		if err != nil {
			return err
		}
		found[n.Name] = rows
	}
	type owned struct {
		entries []node.CatalogEntry
		size    int
	}
	byOwner := map[string]*owned{}
	rows := map[rowKey]sharedRow{}
	for owner, functions := range found {
		byOwner[owner] = &owned{}
		for id, f := range functions {
			internal := meshclient.Internal(f.Opts.Metadata)
			e := node.CatalogEntry{ID: id, Owner: owner, Description: f.Opts.Description, RequestFormat: f.Opts.RequestFormat, ResponseFormat: f.Opts.ResponseFormat, Internal: internal}
			b, _ := json.Marshal(e)
			byOwner[owner].entries = append(byOwner[owner].entries, e)
			byOwner[owner].size += len(b)
			rows[rowKey{owner, id}] = sharedRow{owner, f.Worker, internal}
		}
	}
	entries := []node.CatalogEntry{}
	skipped := []string{}
	total, hubTooLarge := 0, false
	for _, owner := range slices.SortedFunc(maps.Keys(byOwner), meshclient.HubFirst) { // the hub is admitted first
		o := byOwner[owner]
		if len(o.entries) > maxOwnerFunctions || o.size > maxOwnerBytes || total+o.size > maxCatalogBytes {
			log.Printf("mesh share: %s shares %d functions (%d bytes), more than the mesh carries; skipped", owner, len(o.entries), o.size)
			hubTooLarge = hubTooLarge || owner == "hub"
			if owner != "hub" {
				skipped = append(skipped, owner)
			}
			for _, e := range o.entries {
				delete(rows, rowKey{owner, e.ID})
			}
			continue
		}
		total += o.size
		entries = append(entries, o.entries...)
	}
	// Two owners may share one id: the owner breaks the tie, or the revision would move between reads.
	slices.SortFunc(entries, func(a, b node.CatalogEntry) int {
		return cmp.Or(cmp.Compare(a.ID, b.ID), cmp.Compare(a.Owner, b.Owner))
	})
	rev := ""
	if len(entries) > 0 {
		b, _ := json.Marshal(entries)
		sum := sha256.Sum256(b)
		rev = hex.EncodeToString(sum[:])
	}
	states := map[string]meshclient.SharedStatus{}
	for _, w := range share {
		s := meshclient.SharedStatus{Worker: w, State: "not running", Functions: []string{}}
		for id := range found["hub"] {
			if entryOf[id] == w {
				s.State = "too large"
				if !hubTooLarge {
					s.State = "shared"
					s.Functions = append(s.Functions, id)
				}
			}
		}
		slices.Sort(s.Functions)
		states[w] = s
	}
	h.mu.Lock()
	moved := !reflect.DeepEqual(h.shared.workers, states) || !slices.Equal(h.shared.skipped, skipped)
	h.shared = shared{catalog: node.Catalog{Rev: rev, Functions: entries}, rows: rows, workers: states, skipped: skipped}
	h.mu.Unlock()
	if moved {
		h.statusChanged() // the page shows both
	}
	return nil
}
