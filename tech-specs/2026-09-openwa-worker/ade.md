# WhatsApp in ADE

## Intended workflow

An operator opens WhatsApp in ADE, creates a named session, explicitly starts
it, pairs by QR or phone code, and sends a test message to a recipient they
choose. A bounded activity feed displays connection events, incoming messages,
and receipts. No session connects and no message sends merely by opening the
page. Pairing material stays in component memory and expires with the session.

The page uses ADE's existing Schematic components and tokens. A session rail
and one active work surface become list/detail navigation in narrow panes.
The host owns configuration drafts, validation, Save/Reset, and dirty guards.
The worker resolves its actual configuration ID instead of assuming the
legacy `whatsapp` entry; configuration RPCs still target `default`.

## Configuration

The six existing configuration fields get a purpose-built form. Session/media
limits and operation/shutdown timeouts apply to subsequent operations. Changing
the credentials directory requires restart: the running native clients own
SQLite stores and cannot safely migrate their credential files while connected.
Changing event capacity also requires restart because the process owns a
shared broadcast channel plus per-session rings. Pending changes and invalid
reloads are visible through `whatsapp::config-status`; failed loads preserve
the last valid snapshot. Lowering the session limit never removes accounts.

## Delivery

`iii-console-ui` embeds the page and stylesheet in the Rust executable and
registers the assets dynamically. React and shared controls come from ADE;
the QR encoder bundles into the page. Node/pnpm are frontend build tools only.
The delivered worker remains a native binary with no Node runtime.

## Implementation and verification

1. Add live configuration snapshots, schema limits, identity/status endpoints,
   authoritative change subscriptions, and focused reload tests.
2. Add the page, configuration form, QR renderer, request adapters, and UI model
   tests with shared component typing and strict token/style checks.
3. Wire asset embedding, workspace/release build metadata, permissions, and
   updated usage documentation.
4. Run Rust tests and Clippy, frontend tests/build, and an isolated real-engine
   integration test for asset delivery and central configuration reloads.
5. Restart only the existing WhatsApp worker, verify ADE manifest/registration,
   and exercise the real page at wide/narrow widths. Use an offline temporary
   session for verification; pairing and real sends remain operator actions.

## Verification — 2026-09-30

- `cargo test --locked --all-features -- --include-ignored` with `III_BINARY`
  and the final release `WHATSAPP_BINARY`: 61 passing tests. The isolated
  engine checked all 145 public function schemas, namespaced configuration
  identity, form metadata, embedded assets, live tuning, schema rejection,
  authoritative reloads, rejected-read recovery, and pending restart fields.
- `cargo clippy --locked --all-targets --all-features -- -D warnings`, Cargo
  formatting, and `git diff --check` passed.
- `pnpm --dir whatsapp/ui test`: 45 passing tests. The build passed TypeScript,
  scoped CSS/token checks and strict lint with zero errors or warnings. The
  Impeccable detector reported no deterministic findings.
- The release descriptor compiler validated all 82 worker declarations in the
  prescribed container, including WhatsApp's frontend build metadata.
- Live ADE at `http://127.0.0.1:3113/#/worker/whatsapp` served both assets with
  empty manifest warnings. Their contents matched the built files exactly.
- Chromium exercised the actual page: create an offline session, verify Send
  is disabled while stopped, open the instance's configuration, reject zero
  session capacity, save a valid change through the host and observe its live
  application. The original raw configuration was restored and test sessions
  were deleted. Desktop and phone views, drill-in navigation, configuration
  form, light/dark styles, and page overflow were inspected; no browser errors.
- An isolated browser component test used mocked RPC responses to render a
  local QR image and phone code, clear pairing data on connection, and submit
  exactly one explicitly requested message with its normalized recipient. It
  never called a real start/send endpoint and produced no browser errors.
- Only the live WhatsApp process restarted; the other 33 worker PIDs remained
  unchanged. All 34 containers ended ready, configuration `my-project-whatsapp`
  retained the existing credential directory, and the session list was empty.

The executable disables SDK trace payload capture before runtime initialization;
marking traces hidden would only hide their display and still retain secrets.
No real WhatsApp account was paired, and live delivery/receipt interoperability
still needs an operator's account and chosen recipient.

Final release SHA-256:
`b62f15cee0a052ceb6828c3fda2d1a098486f36bf67a6a94db6bd64eb065c1d9`.
