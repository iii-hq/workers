/* Pure formatting helpers shared by every sandbox renderer. No React,
   no DOM access — these are deterministic transforms over the parsed
   sandbox payloads. Kept colocated with the renderers so the
   per-tool views stay terse. */

import type { EnvShape } from './parsers'

/** `1024` → `"1.0 KiB"`. Pin to KiB/MiB/GiB so the unit matches
    the daemon's `INLINE_BUFFER_CAP = 1 MiB` constant in `fs/read.rs`. */
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

/** Octal mode string `"0755"` → POSIX `"rwxr-xr-x"`. For directories
    the caller can prepend the leading `d` (e.g. `${isDir ? 'd' : '-'}${formatMode(mode)}`). */
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

/** Unix seconds → short relative time ("3m ago", "2d ago"). `mtime`
    from the daemon is seconds since epoch (or 0 for unset/unknown). */
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

/** Humanize an age in seconds. Used by `sandbox::list`. */
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

/** Pick a tone for an exit code pill. Null/missing → warn ("unknown"). */
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

/** Infer a syntax-highlight language from a file path. Returns `null`
    when nothing useful matches — callers should fall back to a plain
    `<pre>`. Names mirror `prism-react-renderer` defaults plus the
    handful we manually register in `@/lib/syntax`. */
export function inferLangFromPath(path: string): string | null {
  const lower = path.toLowerCase()
  const ext = lower.match(/\.([a-z0-9]+)$/)?.[1]
  if (!ext) {
    if (lower.endsWith('dockerfile') || lower.includes('/dockerfile'))
      return 'bash'
    if (lower.endsWith('makefile') || lower.includes('/makefile')) return 'bash'
    return null
  }
  switch (ext) {
    case 'js':
    case 'mjs':
    case 'cjs':
      return 'javascript'
    case 'jsx':
      return 'jsx'
    case 'ts':
      return 'typescript'
    case 'tsx':
      return 'tsx'
    case 'json':
    case 'jsonc':
      return 'json'
    case 'py':
    case 'pyi':
      return 'python'
    case 'sh':
    case 'bash':
    case 'zsh':
      return 'bash'
    case 'rs':
      return 'rust'
    case 'go':
      return 'go'
    case 'rb':
      return 'ruby'
    case 'html':
    case 'htm':
      return 'markup'
    case 'css':
      return 'css'
    case 'md':
    case 'mdx':
      return 'markdown'
    case 'yml':
    case 'yaml':
      return 'yaml'
    case 'toml':
      return 'toml'
    default:
      return null
  }
}

/** Map a `lang` field from `sandbox::run` to a highlight language. */
export function langFromRunLang(lang: string): string | null {
  const l = lang.toLowerCase()
  if (l === 'node' || l === 'js' || l === 'javascript') return 'javascript'
  if (l === 'python' || l === 'py') return 'python'
  if (l === 'shell' || l === 'sh' || l === 'bash') return 'bash'
  if (l === 'typescript' || l === 'ts') return 'typescript'
  return null
}

/** Quote an argv slot for terminal display. Single-tokens come through
    bare; anything with whitespace, quotes, or shell metacharacters
    gets single-quoted with embedded single quotes escaped via the
    POSIX `'\''` dance. Output is human-paste-able into a shell. */
export function quoteShellArg(arg: string): string {
  if (arg === '') return "''"
  if (/^[A-Za-z0-9_@%+=:,./-]+$/.test(arg)) return arg
  return `'${arg.replace(/'/g, `'\\''`)}'`
}

/** Render a `(cmd, args | argv)` ExecRequest as a single `cmd args`
    string suitable for the terminal prompt. `argv` wins when present;
    `cmd` shell-line shape (whitespace in `cmd`, no `args`) is left as-is
    because the daemon shlex-splits it server-side. */
export function formatExecCommand(req: {
  cmd?: string | null
  args?: string[] | null
  argv?: string[] | null
}): string {
  const argv = req.argv ?? []
  if (argv.length > 0) {
    return argv.map(quoteShellArg).join(' ')
  }
  const cmd = req.cmd ?? ''
  const args = req.args ?? []
  if (args.length === 0) return cmd
  return [cmd, ...args.map(quoteShellArg)].join(' ')
}

/** Alias used by terminal renderers. */
export const formatCommandLine = formatExecCommand

/** Normalise an `EnvShape` to an array of `[key, value]` tuples,
    sorted by key (the daemon's BTreeMap ordering). */
/** Matches `engine/src/protocol.rs::StreamChannelRef` on the wire. */
export function isStreamChannelRef(
  value: unknown,
): value is { channel_id: string; access_key: string; direction: string } {
  return (
    !!value &&
    typeof value === 'object' &&
    'channel_id' in value &&
    typeof (value as { channel_id: unknown }).channel_id === 'string'
  )
}

const INLINE_BUFFER_CAP = 1024 * 1024

/** Note when streamed output likely hit the daemon inline cap. */
export function streamCapNote(byteLen: number): string | null {
  if (byteLen >= INLINE_BUFFER_CAP) return 'stdout 1.0 MiB cap reached'
  return null
}

export function normaliseEnv(env: EnvShape | undefined): [string, string][] {
  if (!env) return []
  if (Array.isArray(env)) {
    return env
      .map<[string, string]>((kv) => {
        const eq = kv.indexOf('=')
        if (eq < 0) return [kv, '']
        return [kv.slice(0, eq), kv.slice(eq + 1)]
      })
      .sort(([a], [b]) => a.localeCompare(b))
  }
  return Object.entries(env).sort(([a], [b]) => a.localeCompare(b))
}
