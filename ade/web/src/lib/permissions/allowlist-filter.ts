/**
 * Visibility filter for the auto-mode allowlist tree. The functions
 * catalog includes the entire iii bus, including orchestration plumbing
 * (state::*, turn::*, approval::*, …) and runtime services
 * (models-catalog::*, provider-*::*, …) the agent never calls via
 * `agent_trigger`. Showing them in the allowlist is noise and
 * actively misleading — checking `state::set` does nothing useful since
 * agents don't dispatch it.
 *
 * Tier 3 (`agent::trigger`, the recursive-dispatch foot-gun) is
 * intentionally NOT hidden here so operators can still allowlist it if
 * they really want — the destructive-confirm modal protects against
 * accidental clicks.
 */

import type { FunctionEntry } from '@/lib/functions'

/** Namespace prefixes (including `::`) whose entries are always hidden. */
const HIDDEN_PREFIXES: ReadonlyArray<string> = [
  // Tier 1 — orchestration plumbing.
  'state::',
  'session::',
  'session-tree::',
  'session-inbox::',
  'approval::',
  'iii::durable::',
  'ui::',
  // Tier 2 — runtime services / introspection.
  'engine::',
  'functions::',
  'directory::',
  'trigger::',
  'auth::',
  // Provider workers — any registered ids under a provider-* worker
  // are wiring (config, status, capability probes), not agent tools.
  'provider-',
]

/**
 * Exact function ids hidden in addition to the prefix filter. Used for
 * specific lifecycle/event ids that share a namespace with legitimately
 * agent-callable entries (e.g. `agent::trigger` stays visible while the
 * lifecycle events under `agent::` do not).
 */
const HIDDEN_EXACT: ReadonlySet<string> = new Set([
  'agent::before_function_call',
  'agent::after_function_call',
  'agent::turn_end',
  'agent::run_start',
])

function isHidden(id: string): boolean {
  if (HIDDEN_EXACT.has(id)) return true
  for (const prefix of HIDDEN_PREFIXES) {
    if (id.startsWith(prefix)) return true
  }
  return false
}

/**
 * Returns the subset of `entries` that should appear in the allowlist
 * tree. Pure; safe to memoize at the call site.
 */
export function filterAllowlistCandidates(
  entries: ReadonlyArray<FunctionEntry>,
): FunctionEntry[] {
  return entries.filter((e) => !isHidden(e.id))
}
