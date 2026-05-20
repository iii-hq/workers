/**
 * Pure helpers for diffing two pending-approval lists by function_call_id.
 * Used by the translate layer to derive synthetic `fcall-start` /
 * `fcall-end` events from successive `turn_state_changed` updates.
 *
 * No state lives here — the per-session list lives in the closure of
 * `realStream` (one chat = one mirror) and is passed in/out as plain
 * arrays. Keeping this file framework-free makes it unit-testable in
 * isolation.
 */

export interface PendingApproval {
  function_call_id: string
  function_id: string
  args: unknown
}

export interface PendingDiff {
  added: PendingApproval[]
  removed: PendingApproval[]
}

export function diffPending(
  prev: PendingApproval[],
  next: PendingApproval[],
): PendingDiff {
  const prevIds = new Set(prev.map((e) => e.function_call_id))
  const nextIds = new Set(next.map((e) => e.function_call_id))
  return {
    added: next.filter((e) => !prevIds.has(e.function_call_id)),
    removed: prev.filter((e) => !nextIds.has(e.function_call_id)),
  }
}
