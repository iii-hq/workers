# Task 4 report: rbac-proxy worker-name prefix

## Implemented

`on_register_worker` now includes a session's function registration prefix in
the forwarded worker name as `rbac-proxy:<prefix>::<name>`. Sessions without a
prefix continue to use `rbac-proxy:<name>`. Added the requested prefixed and
unprefixed behavior tests in `rbac-proxy/src/interceptor.rs`.

## TDD evidence

### RED

Command, from `rbac-proxy/`:

```text
cargo test --locked --all-features --lib interceptor::tests
```

Result: 49 passed, 1 failed. `prefixed_session_worker_name_carries_prefix`
failed as expected: actual `rbac-proxy:ide`, expected `rbac-proxy:evil::ide`.
The existing implementation omitted the session prefix.

### GREEN

Command, from `rbac-proxy/`:

```text
cargo test --locked --all-features --lib interceptor::tests
```

Result: 50 passed, 0 failed.

## Full worker suite

Command, from `rbac-proxy/`:

```text
cargo test --locked --all-features
```

Result: passed. 112 unit tests, 1 integration test, 1 manifest test, 2 observe
protocol tests, and 3 schema tests passed; doc tests passed with zero tests.

## Files changed

- `rbac-proxy/src/interceptor.rs`
- `.superpowers/sdd/implementation-plan/task-4-report.md`

## Self-review

The name change uses the existing `self.prefix()` accessor and preserves the
existing unprefixed format. The diff does not alter the trigger-type handlers.
`git diff --check` passed. No concerns.
