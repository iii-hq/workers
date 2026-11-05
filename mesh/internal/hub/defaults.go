package hub

// DefaultAllow contains additional project RPC grants. Observed worker bootstrap
// uses the separately scoped configuration calls; shared state and UI calls do
// not pass own-node confinement.
func DefaultAllow() []string { return []string{} }
