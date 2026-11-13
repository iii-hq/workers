package hub

// DefaultAllow contains additional project RPC grants. Observed worker bootstrap
// uses the separately scoped configuration calls; shared state and UI calls do
// not pass own-node confinement. mesh::call and mesh::node::changed are answered
// by the middleware alone and match no engine:: id and no real function.
func DefaultAllow() []string { return []string{meshCallID, nodeChangedID} }
