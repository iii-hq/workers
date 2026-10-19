# observe wire fixtures (vendored)

Copied verbatim from the iii repo's `sdk/fixtures/protocol/observe/`
(iii-hq/iii#2330, at `6d53cbc7c`): the golden JSON for every message and field
iii 0.25's observe adds to the worker protocol. The engine and every SDK
round-trip these files, so they are the shapes a 0.25 peer actually sends.

`tests/observe_protocol.rs` runs each one through the interceptor in the
direction it travels and asserts what the proxy does with it. Every file must
have an expectation there, so a fixture added upstream fails the test until
someone decides how the proxy treats it.

To refresh: copy the directory again from the iii repo and run `cargo test`.
