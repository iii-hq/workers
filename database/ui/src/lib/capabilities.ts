/**
 * Which of the worker's functions this engine actually has.
 *
 * The page no longer carries a client-side catalog fallback: there is one
 * implementation of every read, and it lives in the worker. That removes a
 * whole class of drift, and introduces a hard minimum worker version. An
 * older install must degrade visibly rather than throw "function not found"
 * from four components at once — so a panel whose function is missing is
 * *hidden*, never rendered as a control that always errors.
 *
 * Discovery uses the engine's own `engine::functions::list`. Inventing a
 * `database::capabilities` for this would have been one more function to keep
 * in sync with the real registry.
 */

import type { Host } from '@iii-dev/console-ui'
import { OPTIONAL_FNS, type DbFunction } from './rpc'

export type Capabilities = ReadonlySet<DbFunction>

/** Every optional function, for the common case of an up-to-date worker. */
export const ALL: Capabilities = new Set(OPTIONAL_FNS)

/**
 * Ask the engine which `database::*` functions are registered.
 *
 * Falls back to assuming everything is present when the listing itself is
 * unavailable: a probe that cannot run is not evidence of a missing feature,
 * and hiding the whole page because one introspection call failed would be
 * the worse error. Individual calls still surface their own failures.
 */
export async function probe(host: Host): Promise<Capabilities> {
  try {
    const raw = await host.iii.trigger<unknown>('engine::functions::list', {})
    const ids = collectIds(raw)
    if (ids.size === 0) return ALL
    return new Set(OPTIONAL_FNS.filter((fn) => ids.has(fn)))
  } catch {
    return ALL
  }
}

/**
 * Pull function ids out of whatever shape the listing returns — an array of
 * strings, an array of objects with `id` / `function_id` / `name`, or an
 * object keyed by id. Being permissive here is deliberate: the exact shape is
 * an engine detail, and guessing wrong should not disable the page.
 */
function collectIds(raw: unknown): Set<string> {
  const out = new Set<string>()

  const add = (v: unknown) => {
    if (typeof v === 'string' && v.startsWith('database::')) out.add(v)
  }

  const walk = (node: unknown, depth: number) => {
    if (depth > 4 || node == null) return
    if (typeof node === 'string') return add(node)
    if (Array.isArray(node)) {
      for (const item of node) walk(item, depth + 1)
      return
    }
    if (typeof node === 'object') {
      const rec = node as Record<string, unknown>
      for (const key of ['id', 'function_id', 'functionId', 'name']) {
        add(rec[key])
      }
      // An object keyed by function id.
      for (const key of Object.keys(rec)) add(key)
      for (const key of ['functions', 'data', 'items', 'result']) {
        if (key in rec) walk(rec[key], depth + 1)
      }
    }
  }

  walk(raw, 0)
  return out
}

/** Version hint for the empty state a hidden panel leaves behind. */
export const MIN_VERSION_HINT =
  'this needs a newer database worker — run `iii worker update database`'
