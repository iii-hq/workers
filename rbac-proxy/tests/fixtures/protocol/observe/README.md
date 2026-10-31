# observe wire fixtures (vendored)

Copied verbatim from the iii repo's `sdk/fixtures/protocol/observe/` (branch
`observe-v2`, at `703dc4fa1`): the golden JSON for every message and field
observe adds to the worker protocol, through observe v2. The engine and every
SDK round-trip these files, so they are the shapes a peer actually sends.

`tests/observe_protocol.rs` runs each one through the interceptor in the
direction it travels and asserts what the proxy does with it. Every file must
have an expectation there, so a fixture added upstream fails the test until
someone decides how the proxy treats it.

To refresh: copy the directory again from the iii repo and run `cargo test`.
