/**
 * Pure approval verdict derived from a per-session settings snapshot.
 *
 * Shared by the dispatch-time gate (`consultBefore`) and the parked-call
 * re-evaluation in `function_awaiting_approval`, so both honor the same
 * grants. Returns `'allow'` when the settings alone authorize the call,
 * or `null` when they say nothing (caller falls through to policy / a
 * human decision).
 *
 *   - `mode === 'full'`            → allow everything.
 *   - `approved_always` match      → allow in EVERY mode (a remembered
 *                                    human "approve always" decision).
 *   - `mode === 'auto'` + allowlist match → allow (auto-mode allowlist is
 *                                    dormant outside Auto).
 */

import type { ApprovalSettings } from './types.js';

function listed(
  entries: ApprovalSettings['always_allow'],
  function_id: string,
): boolean {
  return entries.some((entry) => entry.function_id === function_id);
}

export function settingsVerdict(
  settings: ApprovalSettings,
  function_id: string,
): 'allow' | null {
  if (settings.mode === 'full') return 'allow';
  if (listed(settings.approved_always, function_id)) return 'allow';
  if (settings.mode === 'auto' && listed(settings.always_allow, function_id)) {
    return 'allow';
  }
  return null;
}
