# Worker release executors

Release Control is the only supported release interface. GitHub Actions in this
repository are execution endpoints: they accept an exact command, perform a
single bounded effect, and upload factual evidence. They do not choose versions,
channels, dependencies, validation profiles, recovery actions, or schedules.

## Dispatch contract

Every Release Control workflow requires canonical `operation_id` and `step_id`
UUIDs and all effect-specific identities. The run name is
`RC · <kind> · <operation_id> · <step_id>`.

The repository variable `RELEASE_CONTROL_BOT_LOGIN` must be set to
`iii-release-control[bot]`. Dispatches from any other actor are rejected.
Mutating workflows also reject GitHub reruns; recovery creates a new operation
and a new workflow run.

Annotated tags use this shape:

```text
worker: <slug>
version: <exact version>
managed-by: release-control
operation-id: <uuid>
step-id: <uuid>
source-sha: <40 character sha>
maturity: <stable|experimental|alpha|beta>
registry-tag: <next|latest>
experimental: <true|false>
```

Tag creation never starts publication implicitly. Release Control waits for the
tag executor, verifies the tag, and then dispatches the exact publication
workflow.

## Executors

- `create-tag.yml` creates a version commit on main and an annotated tag with a
  manifest CAS guard.
- `create-prerelease-tag.yml` creates an ephemeral versioned commit reachable
  only from the tag.
- `release.yml` builds one immutable worker version and obeys the exact
  `publish_registry` capability supplied by Release Control. `acp` and `lsp`
  publish GitHub artifacts directly to `latest`; they have no Registry
  candidate or promotion stage.
- `candidate-smoke.yml` validates the exact version currently behind `next`.
- `promote-registry.yml` performs only the `latest` Registry CAS.
- `container-alias.yml` moves one exact GHCR channel alias from a pinned digest.
- `reconcile-github-release.yml` applies one exact GitHub Release state.
- `verify-release.yml` reads and verifies the requested release surfaces.
- `harness-e2e-registry.yml` executes an exact stack/scenario/model selection.

## Evidence

Every app-dispatched executor uploads
`execution-result-<operation_id>-<step_id>/execution-result.json`. The schema has
four factual sections: `subject`, `checks`, `effects`, and `outputs`. Workflows
must not emit aggregate release decisions; Release Control derives operation
state from policy and observed facts.

Raw test logs remain ordinary GitHub artifacts. Harness uploads one canonical
`harness-e2e-summary` schema 1 artifact for Release Control ingestion.
There is no GitHub Pages projection or repository-owned release schedule.

## Recovery

Do not rerun a mutating Actions run. Open the failed operation in Release
Control and execute the suggested recovery. The application creates a child
operation containing only the missing effect, such as container alias,
GitHub Release reconciliation, candidate smoke, or surface verification.
