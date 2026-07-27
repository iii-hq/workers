import type { FunctionTriggerMessage } from '@/types/chat'

const now = Date.now()

function base(
  id: string,
  functionId: string,
  input: unknown,
  output?: unknown,
  extra?: Partial<FunctionTriggerMessage>,
): FunctionTriggerMessage {
  return {
    id,
    role: 'function-trigger',
    functionId,
    input,
    ...(output !== undefined ? { output } : {}),
    durationMs: 88,
    createdAt: now,
    ...extra,
  }
}

/* ---------------- submit_result ---------------- */

/* `submit_result` is the output-contract fallback: the call ARGUMENTS are the
   deliverable; the harness consumes the call and it has no response, so these
   fixtures carry no `output`. */

export const submitResultText = base(
  'submit-result-text',
  'submit_result',
  'All three migrations applied cleanly; no rows were dropped.',
)

export const submitResultJson = base('submit-result-json', 'submit_result', {
  status: 'ok',
  migrated: 3,
  skipped: ['2024_legacy_backfill'],
  durationMs: 1432,
})

export const submitResultRunning = base(
  'submit-result-running',
  'submit_result',
  { status: 'ok', migrated: 3 },
  undefined,
  { running: true },
)

export const submitResultEmpty = base(
  'submit-result-empty',
  'submit_result',
  {},
)

/* ---------------- harness::spawn ---------------- */

/** entry-mapper success shape (`functionResultOutput`): { content, details }.
    The dispatcher's `unwrapEnvelope` yields `details` — never the raw harness
    `ResultData { content, is_error, details }`, which never reaches views. */
function resultEnvelope(text: string, details: unknown) {
  return { content: [{ type: 'text' as const, text }], details }
}

/** entry-mapper error shape (`functionResultOutput`, is_error branch). */
function errorEnvelope(code: string, message: string) {
  return {
    error: {
      kind: 'function_error',
      message,
      details: { error: code, message },
      content: [{ type: 'text' as const, text: message }],
    },
  }
}

const ciTriageReport = [
  'Looked at the last 14 runs on `main`.',
  '',
  '- **Failing:** `provider-openai` integration suite (12 of 14 runs)',
  '- **First bad commit:** `4e219fd5` — lockfile bump without the matching feature flag',
  '- **Most likely root cause:** stale `Cargo.lock` pin on `reqwest 0.12.1`, yanked upstream',
  '',
  'Suggested fix: re-run `cargo update -p reqwest` and commit the lockfile.',
].join('\n')

/** Text output contract: the child's final markdown report. */
export const spawnTextDone = base(
  'spawn-text-done',
  'harness::spawn',
  {
    task: 'summarize the failing CI runs on main and propose the single most likely root cause.',
    model: 'claude-sonnet-4-6',
    options: {
      mode: 'agent',
      max_turns: 8,
      thinking_level: 'low',
      system_prompt_strategy: 'enrich',
    },
  },
  resultEnvelope(ciTriageReport, ciTriageReport),
  { durationMs: 48_213 },
)

const auditSummary = {
  status: 'ok',
  failing: 2,
  flaky: ['worker::deploy retry loop', 'http::listen port race'],
  root_cause: 'stale lockfile in provider-openai',
}

/** JSON output contract + a narrowed function policy. */
export const spawnJsonDone = base(
  'spawn-json-done',
  'harness::spawn',
  {
    task: 'audit the last 20 CI runs; return { status, failing, flaky[], root_cause }.',
    model: 'claude-sonnet-4-6',
    options: {
      output: {
        type: 'json',
        schema: { type: 'object', required: ['status'] },
      },
      functions: {
        allow: ['web::fetch', 'sandbox::exec'],
        deny: ['sandbox::fs::rm'],
        expose: 'agent_trigger',
      },
      max_children: 2,
      pending_timeout_ms: 300_000,
    },
  },
  resultEnvelope(JSON.stringify(auditSummary), auditSummary),
  { durationMs: 61_902 },
)

/** Direct-call acknowledgement (task in AgentMessage form). */
export const spawnDirectDone = base(
  'spawn-direct-done',
  'harness::spawn',
  {
    task: {
      role: 'user',
      content: [
        {
          type: 'text',
          text: 'fetch the circuit-breaker article and store a summary under state::set.',
        },
      ],
    },
    model: 'claude-sonnet-4-6',
    session_id: 'console-972e6a3b:fetch-article',
    options: { functions: { allow: ['web::fetch', 'state::set'] } },
  },
  resultEnvelope(
    '{"child_session_id":"s_01HVX2E8Z3TQ","child_turn_id":"t_01HVX2E9AW5K"}',
    {
      child_session_id: 's_01HVX2E8Z3TQ',
      child_turn_id: 't_01HVX2E9AW5K',
    },
  ),
)

/** Depth-guard rejection — renders through SandboxErrorView. */
export const spawnDepthError = base(
  'spawn-depth-error',
  'harness::spawn',
  {
    task: 'spawn another layer of children to parallelize the audit.',
    model: 'claude-sonnet-4-6',
  },
  errorEnvelope(
    'harness/spawn_depth_exceeded',
    'harness/spawn_depth_exceeded: child depth 3 exceeds max_depth 2',
  ),
)

export const spawnRunning = base(
  'spawn-running',
  'harness::spawn',
  {
    task: 'profile the slow session-list query and suggest an index.',
    model: 'claude-sonnet-4-6',
    options: { mode: 'agent', max_turns: 6 },
  },
  undefined,
  { running: true, durationMs: undefined },
)

/** Gated spawn awaiting approval — the preview's policy chips are the point. */
export const spawnPending = base(
  'spawn-pending',
  'harness::spawn',
  {
    task: 'clean up dangling sandboxes older than a day.',
    model: 'claude-sonnet-4-6',
    options: {
      mode: 'agent',
      max_turns: 4,
      thinking_level: 'medium',
      output: { type: 'text' },
      functions: {
        allow: ['sandbox::list', 'sandbox::stop'],
        deny: ['sandbox::fs::rm'],
      },
    },
  },
  undefined,
  { pendingApproval: true },
)

export const harnessFixtures = [
  submitResultText,
  submitResultJson,
  submitResultRunning,
  submitResultEmpty,
  spawnTextDone,
  spawnJsonDone,
  spawnDirectDone,
  spawnDepthError,
  spawnRunning,
  spawnPending,
] as const
