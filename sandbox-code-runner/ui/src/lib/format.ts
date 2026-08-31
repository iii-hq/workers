/**
 * Pure helpers the fleet page and the sandbox family both need. Each is
 * re-exported from the local `format` module that already owns that
 * surface, so call sites keep importing from one place.
 */

/** `1024` → `1.0 KiB`. Pinned to KiB/MiB/GiB so the unit matches the
 *  daemon's 1 MiB inline caps. */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '—'
  if (bytes < 1024) return `${bytes} B`
  const kib = bytes / 1024
  if (kib < 1024) return `${kib.toFixed(kib < 10 ? 1 : 0)} KiB`
  const mib = kib / 1024
  if (mib < 1024) return `${mib.toFixed(mib < 10 ? 1 : 0)} MiB`
  const gib = mib / 1024
  return `${gib.toFixed(gib < 10 ? 1 : 0)} GiB`
}

/** POSIX single-quote an argv slot (embedded `'` via the `'\''` dance).
 *  Single tokens come through bare; the output pastes into a shell. */
export function quoteShellArg(arg: string): string {
  if (arg === '') return "''"
  if (/^[A-Za-z0-9_@%+=:,./-]+$/.test(arg)) return arg
  return `'${arg.replace(/'/g, `'\\''`)}'`
}
