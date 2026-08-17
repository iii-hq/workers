/* Pure formatting helpers shared by every sandbox family renderer. No
   React, no DOM access — deterministic transforms over the parsed
   sandbox payloads. Ported from the console's sandbox family
   (console/web/src/components/chat/sandbox/format.ts) with the
   exit-reason verdict and the 1 MiB truncation detection added. */

import { formatBytes, quoteShellArg } from '../lib/format'
import type { EnvShape } from './parsers'

export { formatBytes }

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

/* --- exit-reason verdict ------------------------------------------------ */

export type ExitTone = 'ok' | 'warn' | 'alert'

export interface ExitReason {
  label: string
  tone: ExitTone
}

/**
 * The daemon's synthetic stderr when the requested binary does not
 * exist in the guest: `exec: <cmd>: not found` (paired with exit 127,
 * the shell convention). Detected explicitly so the verdict can name
 * the failure even on wires that report the exit code oddly.
 */
const EXEC_NOT_FOUND_RE = /\bexec: [^\n]*: not found\b/

export function isSyntheticNotFound(stderr: string): boolean {
  return EXEC_NOT_FOUND_RE.test(stderr)
}

/** `1500` → `"1.5s"`, `30000` → `"30s"`. For the timed-out verdict. */
export function formatTimeoutSecs(durationMs: number): string {
  if (!Number.isFinite(durationMs) || durationMs < 0) return '?s'
  const secs = durationMs / 1000
  const rounded = secs >= 10 ? Math.round(secs) : Math.round(secs * 10) / 10
  return `${rounded}s`
}

/**
 * The single verdict pill for an exec/run card. One pill, one claim:
 *
 *   exit 0                → ok      (clean exit)
 *   timed out @ Ns        → warn    (timed_out + duration_ms)
 *   not found (127)       → alert   (127, or the daemon's synthetic
 *                                    `exec: <cmd>: not found` stderr)
 *   not executable (126)  → alert
 *   no exit               → warn    (null exit code, not timed out)
 *   exit N                → alert
 *
 * Timed-out wins over the exit code: the daemon kills the process, so
 * whatever code rides along describes the kill, not the program.
 */
export function exitReason(resp: {
  exit_code: number | null
  timed_out: boolean
  duration_ms: number
  stderr: string
}): ExitReason {
  if (resp.timed_out) {
    return {
      label: `timed out @ ${formatTimeoutSecs(resp.duration_ms)}`,
      tone: 'warn',
    }
  }
  const exit = resp.exit_code
  if (exit === 0) return { label: 'exit 0', tone: 'ok' }
  if (exit === 127 || isSyntheticNotFound(resp.stderr)) {
    return {
      label: exit === null ? 'not found' : `not found (${exit})`,
      tone: 'alert',
    }
  }
  if (exit === 126) return { label: 'not executable (126)', tone: 'alert' }
  if (exit === null) return { label: 'no exit', tone: 'warn' }
  return { label: `exit ${exit}`, tone: 'alert' }
}

/* --- 1 MiB stream truncation -------------------------------------------- */

/** The daemon's inline buffer cap (`INLINE_BUFFER_CAP` in fs/read.rs);
    exec/run streams are buffered upstream with the same lid. */
const STREAM_TRUNCATION_CAP_BYTES = 1024 * 1024

/**
 * True when a stream's BYTE length reached the daemon's 1 MiB inline
 * cap — i.e. the payload was almost certainly truncated upstream.
 * Cheap paths first: UTF-8 is ≥ 1 and ≤ 3 bytes per UTF-16 code unit
 * (astral pairs average 2/unit), so most strings never get encoded.
 */
export function streamTruncatedAtCap(text: string): boolean {
  if (text.length >= STREAM_TRUNCATION_CAP_BYTES) return true
  if (text.length * 3 < STREAM_TRUNCATION_CAP_BYTES) return false
  return new TextEncoder().encode(text).length >= STREAM_TRUNCATION_CAP_BYTES
}

/* --- languages ----------------------------------------------------------- */

/** Infer a syntax-highlight language from a file path. Returns `null`
    when nothing useful matches — callers should fall back to a plain
    `<pre>`. Names are Prism ids understood by the console's
    CodeHighlight (unknown ids render plain there anyway). */
export function inferLangFromPath(path: string): string | null {
  const lower = path.toLowerCase()
  const ext = lower.match(/\.([a-z0-9]+)$/)?.[1]
  if (!ext) {
    if (lower.endsWith('dockerfile') || lower.includes('/dockerfile')) return 'bash'
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

/** Normalise an `EnvShape` to an array of `[key, value]` tuples,
    sorted by key (the daemon's BTreeMap ordering). */
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
