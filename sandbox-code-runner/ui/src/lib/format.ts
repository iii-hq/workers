/**
 * Pure helpers the fleet page and the sandbox family both need. Each is
 * re-exported from the local `format` module that already owns that
 * surface, so call sites keep importing from one place.
 */

import { formatRelative } from '@iii-dev/console-ui/format'

export { formatBytes } from '@iii-dev/console-ui/format'

/** An age in seconds → `just now`, `42s`, `5m`, `3h`, `2d`. */
export function formatAgeSecs(secs: number): string {
  if (!Number.isFinite(secs) || secs < 0) return '—'
  const now = Date.now()
  return formatRelative(now - secs * 1000, now)
}

/** POSIX single-quote an argv slot (embedded `'` via the `'\''` dance).
 *  Single tokens come through bare; the output pastes into a shell. */
export function quoteShellArg(arg: string): string {
  if (arg === '') return "''"
  if (/^[A-Za-z0-9_@%+=:,./-]+$/.test(arg)) return arg
  return `'${arg.replace(/'/g, `'\\''`)}'`
}
