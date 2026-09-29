/* Shell-specific formatting helpers — no React, no DOM. Bytes, relative
   times, errors and the clipboard come from @iii-dev/console-ui/format. */

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

/** Truncate the middle of a long path/identifier so head and tail stay
    visible. `"a/very/long/path"` → `"a/very/…/path"`. */
export function truncateMiddle(value: string, maxLen = 28): string {
  if (value.length <= maxLen) return value
  const head = Math.ceil((maxLen - 1) / 2)
  const tail = Math.floor((maxLen - 1) / 2)
  return `${value.slice(0, head)}…${value.slice(value.length - tail)}`
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
