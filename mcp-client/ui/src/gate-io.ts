import type { ExtensionIii } from '@iii-dev/console-ui'
import { resolveConfigurationId } from '@iii-dev/console-ui/configuration'
import { errorCode, errorMessage } from '@iii-dev/console-ui/format'
import { blockOf, mergeGateEntry, type Verdict } from './gate-rules'
import type { SaveDeps } from './save'

/** The approval gate's entry, read and written (spec 5.6). An answer the page cannot read is never "no gate",
 * and nothing is written over rules the page could not read. */

export interface GateRead {
  installed: boolean
  id: string | null
  value: unknown
  rules: unknown[] | null
  error?: string
}

const PROBE_SESSION = 'mcp-client-page-probe'
const PROBES_AT_ONCE = 8

async function readAt(iii: ExtensionIii, id: string): Promise<GateRead> {
  try {
    const value = (await iii.call<{ value?: unknown }>('configuration::get', { id, raw: true }))?.value
    // The merge's own check, so the read and the write refuse the same entries in the same words.
    const checked = mergeGateEntry(value, '', [])
    return checked.ok
      ? { installed: true, id, value, rules: checked.value.rules as unknown[] }
      : { installed: true, id, value, rules: null, error: checked.reason }
  } catch (err) {
    return { installed: true, id, value: null, rules: null, error: errorMessage(err) }
  }
}

export async function readGate(iii: ExtensionIii): Promise<GateRead> {
  try {
    return await readAt(iii, await resolveConfigurationId(iii, 'approval-gate'))
  } catch (err) {
    if (errorCode(err)?.toLowerCase() === 'function_not_found') {
      return { installed: false, id: null, value: null, rules: null }
    }
    return { installed: true, id: null, value: null, rules: null, error: errorMessage(err) }
  }
}

const sameIds = (a: string[], b: string[]) => a.length === b.length && a.every((id, i) => id === b[i])

/** Re-read, merge, one `configuration::set`, re-read and compare. `configuration::set` has no compare-and-set:
 * a second write would overwrite whatever somebody else saved since, so a lost write is reported, never retried. */
export async function writeBlock(
  iii: ExtensionIii,
  server: string,
  readIds: string[],
): Promise<{ ok: boolean; reason?: string; written: number; stored: unknown[] | null }> {
  const before = await readGate(iii)
  if (!before.installed) return { ok: false, reason: 'No approval gate on this stack.', written: 0, stored: null }
  if (before.id === null || before.rules === null) return { ok: false, reason: before.error, written: 0, stored: null }
  const merged = mergeGateEntry(before.value, server, readIds)
  if (!merged.ok) return { ok: false, reason: merged.reason, written: 0, stored: null }
  const written = blockOf(merged.value.rules as unknown[], server).ids
  await iii.call('configuration::set', { id: before.id, value: merged.value })
  const after = await readAt(iii, before.id)
  if (after.rules === null) return { ok: false, reason: after.error, written: 0, stored: null }
  if (!sameIds([...blockOf(after.rules, server).ids].sort(), written)) {
    return {
      ok: false,
      reason: 'The approval gate did not keep the rules as written. Its stored rules are shown.',
      written: 0,
      stored: after.rules,
    }
  }
  return { ok: true, written: written.length, stored: after.rules }
}

/** Save, Approve and Remove's pair: a gate that cannot be read rejects, so they stop before writing anything. */
export function gateDeps(iii: ExtensionIii): Pick<SaveDeps, 'removeBlock' | 'blockCount'> {
  const blockCount = async (server: string) => {
    const gate = await readGate(iii)
    if (!gate.installed) return 0
    if (gate.rules === null) throw new Error(gate.error)
    return blockOf(gate.rules, server).ids.length
  }
  return {
    blockCount,
    removeBlock: async (server) => {
      const count = await blockCount(server)
      const { ok, reason } = await writeBlock(iii, server, [])
      return { ok, reason, count }
    },
  }
}

/** The gate's own verdict per function, eight calls in flight over one queue. The probe session has no stored
 * settings, so the answer is the gate's default mode and its rules. A failed call leaves its id out. */
export async function probeVerdicts(iii: ExtensionIii, functionIds: string[]): Promise<Record<string, Verdict>> {
  const out: Record<string, Verdict> = {}
  const queue = [...functionIds]
  const next = async (): Promise<void> => {
    const id = queue.shift()
    if (id === undefined) return
    try {
      out[id] = await iii.call<Verdict>('approval::evaluate', { session_id: PROBE_SESSION, function_id: id })
    } catch {
      // No verdict, which the status reads as "not overridden": never as "allow".
    }
    return next()
  }
  await Promise.all(Array.from({ length: Math.min(PROBES_AT_ONCE, queue.length) }, next))
  return out
}
