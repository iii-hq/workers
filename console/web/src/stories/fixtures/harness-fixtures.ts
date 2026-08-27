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

export const harnessFixtures = [
  submitResultText,
  submitResultJson,
  submitResultRunning,
  submitResultEmpty,
] as const
