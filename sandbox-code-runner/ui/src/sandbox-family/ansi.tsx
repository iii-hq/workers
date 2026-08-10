/**
 * SGR-aware stdout/stderr pane for the sandbox family.
 *
 * The console's first-party family stripped ANSI escapes entirely; this
 * port upgrades the strip to a small SGR interpreter so `cargo test`,
 * `pytest`, `npm` output keep their red/green/yellow meaning — mapped
 * onto the console's design tokens, never raw terminal colors:
 *
 *   red (31/91)      → var(--color-alert)
 *   green (32/92)    → var(--color-ok)
 *   yellow (33/93)   → var(--color-warn)
 *   blue/cyan/magenta (34/36/35 + bright) → var(--color-accent)
 *   bold (1)         → font-weight 600
 *   0 / 39           → reset / default foreground
 *
 * Everything else (background colors, 256-color/truecolor `38;5;…`,
 * underline, cursor movement, OSC titles) is stripped: unknown codes
 * must never leak `\x1b[` bytes into the feed, and this family only
 * claims the four token tones it can honestly map.
 *
 * Streams are clamped to the worker-UI standard (12 lines / 2000 chars
 * behind an expand toggle) BEFORE parsing, so a hostile 5 MB one-liner
 * costs one slice, not a million spans. A stream whose byte length hit
 * the daemon's 1 MiB inline cap gets the `output truncated at 1 MiB`
 * chip — the payload ended because the buffer did, not the program.
 */

import { useMemo, useState } from 'react'
import { streamTruncatedAtCap } from './format'

/* --- the SGR parser ----------------------------------------------------- */

export type AnsiColor = 'alert' | 'ok' | 'warn' | 'accent'

export interface AnsiSpan {
  text: string
  color?: AnsiColor
  bold?: boolean
}

/** SGR fg code → token tone. Black/white (30/37/90/97) stay unmapped:
    the pane's own ink is the only honest "default" in both themes. */
const SGR_COLOR: Record<number, AnsiColor | undefined> = {
  31: 'alert',
  91: 'alert',
  32: 'ok',
  92: 'ok',
  33: 'warn',
  93: 'warn',
  34: 'accent',
  94: 'accent',
  35: 'accent',
  95: 'accent',
  36: 'accent',
  96: 'accent',
}

/* One pass over the string: the first alternative captures SGR parameter
   lists (`ESC [ params m`); the second eats any other CSI sequence; the
   third eats OSC (`ESC ] … BEL` or `ESC ] … ESC \`). Non-SGR matches
   are stripped without touching the style state. */
// biome-ignore lint/suspicious/noControlCharactersInRegex: parsing ANSI escape sequences.
const ANSI_RE = /\x1b\[([0-9;]*)m|\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)?/g

/**
 * Parse `input` into styled spans. Total: never throws, strips what it
 * does not understand, and handles malformed input (dangling escapes
 * simply fail the regex and render as-is). Linear in input length.
 */
export function parseAnsi(input: string): AnsiSpan[] {
  const spans: AnsiSpan[] = []
  let color: AnsiColor | undefined
  let bold = false
  let last = 0

  const push = (text: string) => {
    if (text.length === 0) return
    const prev = spans[spans.length - 1]
    if (prev && prev.color === color && !!prev.bold === bold) {
      prev.text += text
      return
    }
    spans.push({ text, ...(color ? { color } : {}), ...(bold ? { bold: true } : {}) })
  }

  ANSI_RE.lastIndex = 0
  for (let match = ANSI_RE.exec(input); match !== null; match = ANSI_RE.exec(input)) {
    push(input.slice(last, match.index))
    last = match.index + match[0].length
    const params = match[1]
    if (params === undefined) continue // non-SGR escape: stripped
    // An empty parameter list (`ESC[m`) means reset, per SGR.
    const codes = params === '' ? [0] : params.split(';').map((p) => (p === '' ? 0 : Number.parseInt(p, 10)))
    for (let i = 0; i < codes.length; i++) {
      const code = codes[i]
      if (Number.isNaN(code)) continue
      if (code === 38 || code === 48 || code === 58) {
        // Extended-color introducer (fg/bg/underline): its arguments are
        // parameters, not codes — consume them (5;n or 2;r;g;b) so
        // `38;5;31` never reads 31 as a standalone red.
        const mode = codes[i + 1]
        i += mode === 5 ? 2 : mode === 2 ? 4 : 0
        continue
      }
      if (code === 0) {
        color = undefined
        bold = false
      } else if (code === 1) {
        bold = true
      } else if (code === 39) {
        color = undefined
      } else if ((code >= 30 && code <= 37) || (code >= 90 && code <= 97)) {
        color = SGR_COLOR[code]
      }
      // Everything else: stripped, no style change.
    }
  }
  push(input.slice(last))
  return spans
}

/* --- rendering ----------------------------------------------------------- */

/** Styled spans for one already-clamped chunk of stream text. */
export function AnsiText({ text }: { text: string }) {
  const spans = useMemo(() => parseAnsi(text), [text])
  if (spans.length === 1 && !spans[0].color && !spans[0].bold) {
    return <>{spans[0].text}</>
  }
  return (
    <>
      {spans.map((span, i) => {
        if (!span.color && !span.bold) {
          // biome-ignore lint/suspicious/noArrayIndexKey: spans are static per render.
          return <span key={i}>{span.text}</span>
        }
        const cls = [span.color ? `cr-fam-ansi-${span.color}` : '', span.bold ? 'cr-fam-ansi-bold' : '']
          .filter(Boolean)
          .join(' ')
        return (
          // biome-ignore lint/suspicious/noArrayIndexKey: spans are static per render.
          <span key={i} className={cls}>
            {span.text}
          </span>
        )
      })}
    </>
  )
}

/* The worker-UI standard stream clamp (lib/shared.tsx uses the same
   values for the worker's own cards): lines behind a toggle, plus a
   character ceiling for the one-line blob a minifier emits. */
const STREAM_CLAMP_LINES = 12
const STREAM_CLAMP_CHARS = 2000

function StreamBlock({ text, tone }: { text: string; tone: 'out' | 'err' }) {
  const [expanded, setExpanded] = useState(false)
  // Keyed by text so the expand toggle never re-runs the byte-length
  // check or the line split on a large stream.
  const truncated = useMemo(() => streamTruncatedAtCap(text), [text])
  const lines = useMemo(() => text.split('\n'), [text])
  const long = lines.length > STREAM_CLAMP_LINES || text.length > STREAM_CLAMP_CHARS
  const collapsed = long && !expanded
  const shown = collapsed ? lines.slice(0, STREAM_CLAMP_LINES).join('\n').slice(0, STREAM_CLAMP_CHARS) : text

  return (
    <div className={`cr-fam-stream ${tone}`}>
      {truncated ? <span className="cr-fam-chip warn cr-fam-trunc">output truncated at 1 MiB</span> : null}
      <pre className="cr-fam-stream-pre">
        <code>
          <AnsiText text={shown} />
        </code>
      </pre>
      {long ? (
        <button type="button" className="cr-fam-toggle" onClick={() => setExpanded((v) => !v)}>
          {collapsed ? `expand · ${lines.length} lines, ${text.length} chars` : 'collapse'}
        </button>
      ) : null}
    </div>
  )
}

interface AnsiOutputProps {
  stdout?: string
  stderr?: string
}

/**
 * Stream-coloured stdout/stderr pane. Sandbox exec is buffered (no
 * incremental writes are surfaced), so stdout stacks above stderr with
 * distinct tones rather than interleaving by timestamp (which the wire
 * shape doesn't carry).
 */
export function AnsiOutput({ stdout, stderr }: AnsiOutputProps) {
  const hasStdout = !!stdout && stdout.length > 0
  const hasStderr = !!stderr && stderr.length > 0

  if (!hasStdout && !hasStderr) {
    return <div className="cr-fam-note-ghost">· no output</div>
  }

  return (
    <div className="cr-fam-streams">
      {hasStdout ? <StreamBlock text={stdout ?? ''} tone="out" /> : null}
      {hasStderr ? <StreamBlock text={stderr ?? ''} tone="err" /> : null}
    </div>
  )
}
