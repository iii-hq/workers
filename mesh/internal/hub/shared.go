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
	"strings"

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
	rows    map[string]sharedRow               // catalog entries by hub-engine id
	workers map[string]meshclient.SharedStatus // the share entries read, by worker
	skipped []string                           // machines whose functions do not fit
}
type sharedRow struct{ owner, worker string }

func bridgeWorker(machine string) string { return "rbac-proxy:" + machine + "::mesh-bridge" }

// refreshShared reads the catalog with no lock held and swaps it in: the hub's
// shared workers' functions and those of every machine on the hub's version.
func (h *Hub) refreshShared(ctx context.Context) error {
	engine := h.engineRPC()
	if engine == nil || h.Coordinator == nil {
		return errors.New("hub engine is not connected")
	}
	share := h.Config().Share
	joined, current := map[string]bool{}, map[string]bool{}
	workers := slices.Clone(share)
	for _, n := range h.Coordinator.Nodes() {
		if n.Name == "hub" {
			continue
		}
		joined[n.Name] = true
		if n.Version == h.Version {
			current[n.Name] = true
			workers = append(workers, bridgeWorker(n.Name))
		}
	}
	found := map[string]meshclient.Function{}
	conflict := map[string]bool{}
	kept := map[string]int{}
	if len(workers) > 0 { // an empty filter lists every worker
		var err error
		found, err = meshclient.Discover(ctx, engine, h.Namespace, workers, func(r meshclient.Row) bool {
			if r.Namespace != h.Namespace {
				return false
			}
			owner, local, _ := strings.Cut(r.FunctionID, "::")
			hubRow := slices.Contains(share, r.WorkerName)
			keep := hubRow && meshclient.Bridgeable(r.FunctionID)
			if joined[owner] {
				// The owner comes from the id, proven by the bridge's worker name; a hub
				// worker under a machine's name could be that machine's unnamed worker.
				conflict[r.WorkerName] = conflict[r.WorkerName] || hubRow
				keep = current[owner] && r.WorkerName == bridgeWorker(owner) && meshclient.Bridgeable(local)
			} else {
				owner = "hub"
			}
			if keep {
				kept[owner]++
			}
			// One row past the cap still skips the owner below, and no schema past it is read.
			return keep && kept[owner] <= maxOwnerFunctions+1
		}, nil)
		if err != nil {
			return err
		}
	}
	type owned struct {
		entries []node.CatalogEntry
		size    int
	}
	byOwner := map[string]*owned{}
	rows := map[string]sharedRow{}
	for id, f := range found {
		owner := "hub"
		if !slices.Contains(share, f.Worker) {
			owner, _, _ = strings.Cut(id, "::")
		} else if conflict[f.Worker] {
			continue
		}
		e := node.CatalogEntry{ID: id, Owner: owner, Description: f.Opts.Description, RequestFormat: f.Opts.RequestFormat, ResponseFormat: f.Opts.ResponseFormat}
		b, _ := json.Marshal(e)
		if byOwner[owner] == nil {
			byOwner[owner] = &owned{}
		}
		byOwner[owner].entries = append(byOwner[owner].entries, e)
		byOwner[owner].size += len(b)
		rows[id] = sharedRow{owner, f.Worker}
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
				delete(rows, e.ID)
			}
			continue
		}
		total += o.size
		entries = append(entries, o.entries...)
	}
	slices.SortFunc(entries, func(a, b node.CatalogEntry) int { return cmp.Compare(a.ID, b.ID) })
	rev := ""
	if len(entries) > 0 {
		b, _ := json.Marshal(entries)
		sum := sha256.Sum256(b)
		rev = hex.EncodeToString(sum[:])
	}
	states := map[string]meshclient.SharedStatus{}
	for _, w := range share {
		s := meshclient.SharedStatus{Worker: w, State: "not running", Functions: []string{}}
		for id, f := range found {
			if f.Worker == w && !conflict[w] {
				s.State = "too large"
				if !hubTooLarge {
					s.State = "shared"
					s.Functions = append(s.Functions, "hub::"+id)
				}
			}
		}
		if conflict[w] {
			s.State = "name conflict"
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
