/* Pure formatting helpers shared by the shell renderers and the explorer
   page. No React, no DOM access — deterministic transforms over parsed
   wire payloads. Ported from the console's sandbox/format.ts when the
   shell function-trigger family moved into this worker's injected UI. */

/** `1024` → `"1.0 KiB"`. Pin to KiB/MiB/GiB so the unit matches the
    worker's size-cap constants. */
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

/** Octal mode string `"0755"` → POSIX `"rwxr-xr-x"`. For directories the
    caller can prepend the leading `d`. */
export function formatMode(mode: string): string {
  // Tolerate `"0755"`, `"755"`, leading `"o"`, or junk. Extract the last
  // 3 octal digits and decode each; fall back to the original string if
  // it doesn't parse.
  const digits = mode.match(/[0-7]{3}$/)?.[0]
  if (!digits) return mode
  const bits = ['r', 'w', 'x'] as const
  let out = ''
  for (const ch of digits) {
    const n = Number.parseInt(ch, 10)
    for (let i = 0; i < 3; i++) {
      out += n & (4 >> i) ? bits[i] : '-'
    }
  }
  return out
}

/** Unix seconds → short relative time ("3m ago", "2d ago"). `mtime` from
    the worker is seconds since epoch (or 0 for unset/unknown). */
export function formatMtime(unixSecs: number, now = Date.now()): string {
  if (!unixSecs || unixSecs <= 0) return '—'
  const deltaSecs = Math.max(0, Math.floor(now / 1000 - unixSecs))
  if (deltaSecs < 60) return deltaSecs <= 1 ? 'just now' : `${deltaSecs}s ago`
  const mins = Math.floor(deltaSecs / 60)
  if (mins < 60) return `${mins}m ago`
  const hours = Math.floor(mins / 60)
  if (hours < 24) return `${hours}h ago`
  const days = Math.floor(hours / 24)
  if (days < 30) return `${days}d ago`
  const months = Math.floor(days / 30)
  if (months < 12) return `${months}mo ago`
  return `${Math.floor(months / 12)}y ago`
}

/** Truncate the middle of a long path/identifier so head and tail stay
    visible. `"a/very/long/path"` → `"a/very/…/path"`. */
export function truncateMiddle(value: string, maxLen = 28): string {
  if (value.length <= maxLen) return value
  const head = Math.ceil((maxLen - 1) / 2)
  const tail = Math.floor((maxLen - 1) / 2)
  return `${value.slice(0, head)}…${value.slice(value.length - tail)}`
}

/** Humanize an age in seconds ("2m", "3h"). */
export function formatAgeSecs(secs: number): string {
  if (!Number.isFinite(secs) || secs < 0) return '—'
  if (secs < 60) return `${secs}s`
  const mins = Math.floor(secs / 60)
  if (mins < 60) return `${mins}m`
  const hours = Math.floor(mins / 60)
  if (hours < 24) return `${hours}h`
  const days = Math.floor(hours / 24)
  return `${days}d`
}

/** Pick a tone for an exit code pill. Null/missing → warn ("no exit"). */
export function pillForExit(exitCode: number | null | undefined): {
  label: string
  tone: 'accent' | 'warn' | 'alert' | 'default'
} {
  if (exitCode === 0) return { label: 'exit 0', tone: 'accent' }
  if (exitCode === null || exitCode === undefined) {
    return { label: 'no exit', tone: 'warn' }
  }
  return { label: `exit ${exitCode}`, tone: 'alert' }
}

/** Quote an argv slot for terminal display. Single-tokens come through
    bare; anything with whitespace, quotes, or shell metacharacters gets
    single-quoted with embedded single quotes escaped via the POSIX
    `'\''` dance. Output is human-paste-able into a shell. */
export function quoteShellArg(arg: string): string {
  if (arg === '') return "''"
  if (/^[A-Za-z0-9_@%+=:,./-]+$/.test(arg)) return arg
  return `'${arg.replace(/'/g, `'\\''`)}'`
}

/** A human-readable message from ANY thrown value. The bus client throws
    structured payloads (plain objects), which `String(err)` renders as
    the useless `[object Object]` — surface their JSON instead. */
export function errorMessage(err: unknown): string {
  if (err instanceof Error) return err.message
  if (typeof err === 'string') return err
  if (err && typeof err === 'object') {
    const msg = (err as Record<string, unknown>).message
    if (typeof msg === 'string' && msg.length > 0) return msg
    try {
      return JSON.stringify(err)
    } catch {
      return String(err)
    }
  }
  return String(err)
}
