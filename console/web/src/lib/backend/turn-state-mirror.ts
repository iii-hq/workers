/**
 * Derive the canonical pending-approvals list from a turn_state record.
 * The orchestrator parks in `function_awaiting_approval` with a list of
 * pending function calls on the record itself; this function is the
 * single place that knows the schema, so the rest of the FE can stay
 * agnostic of the orchestrator's internal shape.
 */

import type { PendingApproval } from './pending-approvals-store'

const PARKING_STATE = 'function_awaiting_approval'

export function pendingApprovalsFromTurnState(
  record: unknown,
): PendingApproval[] {
  if (!record || typeof record !== 'object') return []
  const r = record as Record<string, unknown>
  if (r.state !== PARKING_STATE) return []
  const raw = r.awaiting_approval
  if (!Array.isArray(raw)) return []
  const out: PendingApproval[] = []
  for (const entry of raw) {
    if (!entry || typeof entry !== 'object') continue
    const e = entry as Record<string, unknown>
    if (typeof e.function_call_id !== 'string') continue
    if (typeof e.function_id !== 'string') continue
    out.push({
      function_call_id: e.function_call_id,
      function_id: e.function_id,
      args: e.args ?? {},
    })
  }
  return out
}
