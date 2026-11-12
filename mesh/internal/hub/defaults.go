package hub

// DefaultAllow contains additional project RPC grants. Observed worker bootstrap
// uses the separately scoped configuration calls; shared state and UI calls do
// not pass own-node confinement. mesh::call is answered by the middleware alone
// and matches no engine:: id and no real function.
func DefaultAllow() []string { return []string{meshCallID} }
