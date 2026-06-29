import type { FunctionCallMessage } from '@/types/chat'

const now = Date.now()

/* Synthetic harness tools — `submit_result` is the output-contract fallback.
   The call ARGUMENTS are the deliverable; the harness consumes the call and
   it has no response, so these fixtures carry no `output`. */
function base(
  id: string,
  input: unknown,
  extra?: Partial<FunctionCallMessage>,
): FunctionCallMessage {
  return {
    id,
    role: 'function-call',
    functionId: 'submit_result',
    input,
    durationMs: 88,
    createdAt: now,
    ...extra,
  }
}

/* ---------------- submit_result ---------------- */

export const submitResultText = base(
  'submit-result-text',
  'All three migrations applied cleanly; no rows were dropped.',
)

export const submitResultJson = base('submit-result-json', {
  status: 'ok',
  migrated: 3,
  skipped: ['2024_legacy_backfill'],
  durationMs: 1432,
})

export const submitResultRunning = base(
  'submit-result-running',
  { status: 'ok', migrated: 3 },
  { running: true },
)

export const submitResultEmpty = base('submit-result-empty', {})

export const harnessFixtures = [
  submitResultText,
  submitResultJson,
  submitResultRunning,
  submitResultEmpty,
] as const
