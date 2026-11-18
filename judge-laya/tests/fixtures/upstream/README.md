# laya's recorded routing vectors

Copied verbatim from [NandhaKishorM/laya](https://github.com/NandhaKishorM/laya)
at commit `b1d656952f0f` (release 0.4.35), `laya-rust/tests/golden/routing/`.
laya recorded them from its Python reference (laya 0.4.17, per that directory's
`meta.json`), and its Rust, .NET and TypeScript ports are tested against them.
Apache-2.0, like laya.

| File | sha256 | Read by |
|---|---|---|
| `lang_probe.json` (261 cases: `state` to `analyse`) | `7389997cbb2974b018cc465ef34e6581caf070a4537d2d99a4a1982b797fb1d5` | `tests/lang.rs` |
| `route_probe.json` (159 cases: router options and `state` to the routed checkpoint) | `4e78af005b93d0e45527e2708d05f10816de52428f6279baf50e7c37784943e4` | `tests/client.rs` |

Do not edit them; replace them with a newer laya's files instead.
