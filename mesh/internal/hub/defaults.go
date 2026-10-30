package hub

// DefaultAllow contains additional project RPC grants. Observed worker bootstrap
// uses the separately scoped configuration calls; shared state and UI calls do
// not pass own-node confinement.
func DefaultAllow() []string { return []string{} }

// DefaultTriggerTypes contains additional shared trigger types. Configuration
// already has an ownership validator; Console assets and harness hooks can reach
// other nodes' data, so observed registrations do not justify a shared grant.
func DefaultTriggerTypes() []string { return []string{} }
