# security-scan

`security-scan` accepts manual and operator-scheduled review requests for configured repositories and queues a report-only security analysis of an exact Git commit. It creates an isolated checkout of the **full tree at that commit**, constrains Harness to read-only code functions, validates the structured result, and never applies a suggested change. The SHA is not a commit-diff review and not a scan of git history. Omit `target_sha` (or leave the Console SHA field blank) to analyze the entire repository at HEAD.

## Install

```bash
iii worker add security-scan
```

Analysis requires the Harness stack. GitHub issue and draft fix-PR actions also require `approval-gate` to be live; those mutations stay closed until a user approves each one.

```bash
iii worker add harness
iii worker add approval-gate
```

The worker composes existing iii infrastructure rather than implementing local substitutes: private compare-and-set records live in `state`, durable steps run through `queue`, exact checkouts come from `worktree`, configured schedules bind through the `cron` dependency, analysis runs through `harness`, and GitHub publication is held by `approval-gate`.

## Quickstart

Request a scan using a configured repository id and a full commit SHA:

```bash
iii trigger security-scan::request \
  repository=iii-hq/iii \
  target_sha="$(git -C /srv/repos/iii rev-parse HEAD)" \
  mode=scan \
  model=deepseek::deepseek-v4-flash
```

The request returns immediately:

```json
{
  "run_id": "sec_...",
  "status": "queued",
  "deduplicated": false
}
```

Submitting the same repository, commit, mode, and model again returns the same run id with `deduplicated: true`. Omit `target_sha` to resolve HEAD and analyze the entire repository. Omit `model` to use the operator `analysis.model` (and its provider). A Console scan from the sidebar runs on the model picked in its analysis-model control; that picker defaults to following the composer catalog id of the open chat, such as `deepseek::deepseek-v4-flash`. A retryable failed run is restarted as a new attempt under that same id. If the first queue wake fails, the durable queued checkpoint remains available to the recovery sweep. Use `mode=suggest` to include minimal patch suggestions in the report; suggestions remain text and are never applied.

Read the current status or completed report:

```bash
iii trigger security-scan::read run_id=sec_...
```

Read the persisted GitHub reconciliation snapshot, or explicitly refresh it:

```bash
iii trigger security-scan::reconciliation run_id=sec_...
iii trigger security-scan::reconciliation run_id=sec_... refresh=true limit=50
```

The Harness count and GitHub alert counts answer different questions and are never added together. Harness findings are validated against the requested exact commit. Dependabot is a repository default-branch snapshot, while code scanning is a repository snapshot whose latest instances may refer to commits other than the requested SHA. A Harness count of 3 and GitHub source counts totaling 221 therefore remain 3 exact-commit findings and 221 GitHub records, not 224 unique findings.

Each GitHub source reports its own scope and collection status: `complete`, `partial`, `unavailable`, `authentication_required`, `permission_denied`, `disabled`, `not_configured`, or `not_collected`. `complete` with `record_count: 0` is a successful empty collection. A null count means no usable count was collected and is not equivalent to zero. Records are deduplicated only by GitHub source and alert number; v1 does not claim semantic matches between model-authored Harness findings and typed GitHub alerts.

The default `refresh=false` reads the last sanitized snapshot without calling GitHub. Before the first collection it returns `not_collected`, or `not_configured` when no GitHub mapping exists. `refresh=true` queries Dependabot and code scanning, replaces the persisted snapshot, and then applies source, severity, lifecycle, cursor, and limit filters. One unavailable source does not fail the whole response; its status explains the missing count.

List recent runs, optionally filtered by repository or status:

```bash
iii trigger security-scan::list repository=iii-hq/iii status=completed limit=50
```

## Console page

When `security-scan` and Console are connected, open `#/ext/security-scan` to browse persisted run history and inspect a selected report. The page shows the exact repository and commit, current pipeline status, evidence and remediation for each finding, and suggested patches in `suggest` mode. Suggested patches remain read-only until you explicitly create a draft fix PR and approve the GitHub mutation. The sidebar form reviews the full tree at the pasted SHA. Its analysis-model picker lists the live router catalog and pins one model for the scan; left on `follow chat` it takes the model selected in the open chat composer, and falls back to the operator `analysis.model` when that chat has none. The catalog is re-read on the router's `router::models::changed` fan-out, so a credential added or a provider removed updates the list without a reload.

Each Harness finding can start an approval-gated GitHub issue. Completed `suggest` findings that include a patch can also start an isolated draft fix PR. GitHub reconciliation alerts are a separate snapshot and cannot start exact-commit Harness actions.

Run updates arrive through the `security-scan:runs` stream. The stream is a refresh doorbell rather than the source of truth: each frame makes the page refetch `security-scan::list` and `security-scan::read`. Nothing is polled. The page re-reads on three other events instead — the socket reconnecting, the tab becoming visible, and the refresh control — so a dropped frame delays convergence until the next event rather than stranding the view.

A completed report records coverage separately for vulnerabilities, dependencies, secrets, and supply-chain review. An area can be assessed, not assessed with a reason, or unknown for reports created before coverage tracking. Zero findings are never presented as proof that the code is vulnerability-free.

GitHub reconciliation and GitHub source links require the explicit operator-verified `github.full_name` mapping. The worker never infers a GitHub repository from the security-scan repository id.

For local UI development:

```bash
pnpm --dir security-scan/ui build
III_SECURITY_SCAN_UI_WATCH=security-scan/ui/dist cargo run --manifest-path security-scan/Cargo.toml
```

The page header's configure control opens the console's own worker-configuration dialog for this worker: the analysis budgets (`max_turns` and the token and cost ceilings), the operator `analysis.model`, and the repository allowlist. A console that predates that shared dialog navigates to the workers tab instead.

## Configuration

Repositories are an operator-owned allowlist. Callers choose an id, not an arbitrary filesystem path or URL.

```yaml
repositories:
  - id: iii-hq/iii       # stable id accepted by security-scan::request
    path: /srv/repos/iii # local Git repository owned by the operator
    github:               # optional; required for GitHub reconciliation
      full_name: iii-hq/iii # exact owner/name for this checkout
    schedule:             # optional; omit to disable automation for this repository
      expression: "0 0 3 * * *" # second minute hour day month weekday [year], UTC
      target_ref: refs/heads/main # resolved locally when each fire occurs
      mode: scan           # scan or suggest
analysis:
  model: provider/model-id # required model from the live router catalog
  provider: provider-id    # optional explicit provider
  max_turns: 4             # maximum Harness generations
  max_output_tokens: 8000  # ceiling for one generation
  max_total_tokens: 50000  # ceiling for the complete review
  max_cost_usd: 2.0        # optional spend ceiling
archive:                   # optional; JSON copies of run records in `storage`
  bucket: security-scan    # worker-facing bucket name
  prefix: runs             # object key prefix, default runs/
```

The shipped configuration leaves `analysis.model` empty and `repositories: []` unchanged. Set a model and at least one repository before requesting a scan; the empty repository allowlist rejects every request.

`github.full_name` is optional so existing local-only repositories remain valid, but it must be configured explicitly as `owner/name` before refresh is enabled for that repository. The `github` worker needs an authenticated GitHub CLI session or `GH_TOKEN` with permission to read Dependabot and code-scanning alerts for the mapped repository. Authentication, permission, and disabled-feature failures are stored only as sanitized source statuses; credentials and raw dependency payloads are never persisted or returned.

Each repository has at most one schedule, so the repository id is also its unique schedule identity. The expression must use six fields starting with seconds, with an optional seventh year field. Cron evaluation is UTC. Fires missed while `cron` or `security-scan` is stopped are skipped and are not replayed.

At fire time the internal handler uses trigger metadata only to find this operator-owned configuration. It resolves `target_ref` with a bounded local `git rev-parse` call, does not fetch, requires one lowercase full 40-character commit SHA, and submits that SHA through the same `security-scan::request` path used manually. Repeated fires that resolve to the same repository, commit, and mode therefore return the existing run instead of creating duplicate work.

Configuration is loaded at worker startup in this MVP. Restart `security-scan` after changing repositories, GitHub mappings, schedules, analysis settings, or archive settings. The worker manifest starts `github` and `cron` as dependencies. If a manually assembled stack starts `security-scan` before a cron trigger owner is available, manual scans remain available and the recovery loop binds each configured schedule once `cron` appears.

## Persistence

The Console Scan runs list is served from `state`. Point that worker at a file-backed or Redis adapter so history survives engine restarts. `store_method: in_memory` drops every run on shutdown.

Optional JSON copies of each run record are written through `storage::putObject` when `archive.bucket` is set. On boot, missing runs are imported from that bucket into `state` using `storage::getObject` and `runs/manifest.json` (`storage` does not list objects). Configure the bucket on the `storage` worker first:

```yaml
providers:
  local:
    data_dir: ./data/storage
buckets:
  security-scan:
    provider: local
    bucket: security-scan
```

The local provider spawns a rustfs sidecar (`iii worker add storage`). Set `$RUSTFS_BIN` or put `rustfs` on `PATH`. JSON copies are a backup; the Console Scan runs list still comes from `state`.

## Safety boundary

The worker accepts only 40-character commit SHAs, verifies the materialized checkout matches the requested commit, and disables ignored-file provisioning for scanner worktrees so local `.env`, dependency, and cache files are not copied into the review scope. The Harness scan turn can discover function contracts and call only `coder::info`, `coder::tree`, `coder::list-folder`, `coder::read-file`, and `coder::search`. It cannot run repository code, access the network, mutate files, update state, or start another agent.

Dependency sessions use private random identities rather than the public run id. Structured output is rejected if it exposes the internal checkout root or high-confidence credential material. Terminal scanner worktrees are removed through the existing `worktree` worker.

GitHub issue and draft fix-PR actions use a separate Harness session after an explicit user request. Issue sessions may call only `github::issue::create`. Fix sessions use an exact-SHA worktree with scoped file writes, explicit git commands, and `github::pr::create`. Both require `approval::gate` to be live; GitHub publication, branch push, and PR creation stay held until the user approves them. Fix PRs open as drafts and never merge automatically.

The public MVP exposes `security-scan::request`, `security-scan::read`, `security-scan::list`, `security-scan::reconciliation`, `security-scan::action`, and `security-scan::action-read`. `security-scan::execute`, `security-scan::action-execute`, `security-scan::on-turn-completed`, and `security-scan::on-schedule` are internal worker functions. Scan analysis still does not apply, commit, push, comment, review, merge, or dismiss alerts on its own.

This first phase is the bounded investigation layer. A later phase will feed it deterministic, pinned SAST, dependency, and secret-scanner candidates before Harness analysis, following the same candidate-discovery then evidence-review split used by DeepSec.
