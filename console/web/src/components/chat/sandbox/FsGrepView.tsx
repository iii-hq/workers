import type * as React from 'react'
import {
  fsGrepRequestSchema,
  fsGrepResponseSchema,
  safeParseResponse,
} from './parsers'
import { Chip, FooterPill } from './terminal/Terminal'

interface FsGrepViewProps {
  input: unknown
  output: unknown
}

export function FsGrepView({ input, output }: FsGrepViewProps) {
  const req = fsGrepRequestSchema.safeParse(input)
  if (!req.success) return null
  const resp = safeParseResponse(fsGrepResponseSchema, output)
  if (!resp) return null
  const { matches, truncated } = resp

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="bg-paper-2 border-b border-rule-2 px-3 py-2 flex flex-wrap items-center gap-1.5">
        <Chip label="path">{req.data.path}</Chip>
        <Chip label="pattern">{req.data.pattern}</Chip>
        {req.data.ignore_case ? <Chip>case-insensitive</Chip> : null}
        <FooterPill tone={matches.length > 0 ? 'default' : 'warn'}>
          {`${matches.length} ${matches.length === 1 ? 'match' : 'matches'}`}
        </FooterPill>
        {truncated ? <FooterPill tone="warn">truncated</FooterPill> : null}
      </div>

      {matches.length === 0 ? (
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost">
          · no matches
        </div>
      ) : (
        <div className="font-mono text-[12px] leading-[1.55]">
          {matches.map((m) => (
            <div
              /* path+line+content is collision-resistant in practice (two
                 hits on the same line of the same file produce identical
                 FsMatch records on the wire today). React will only warn
                 if the daemon ever sends true duplicates, which would
                 itself be a wire-shape bug worth surfacing. */
              key={`${m.path}:${m.line}:${m.content}`}
              className="border-b border-rule-2 last:border-b-0 px-3 py-1.5"
            >
              <div className="text-ink-faint">
                <span className="text-accent">{m.path}</span>
                <span className="text-ink-ghost">:</span>
                <span className="tabular-nums">{m.line}</span>
              </div>
              <pre className="text-ink whitespace-pre-wrap break-words m-0 mt-0.5">
                <code>
                  {renderWithHighlight(
                    m.content,
                    req.data.pattern,
                    !!req.data.ignore_case,
                  )}
                </code>
              </pre>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

/** Best-effort substring/regex highlight. The daemon uses the Rust
    `regex` crate; JS regex is a superset for the simple cases agents
    use (TODO|FIXME, identifiers). Falls back to substring matching
    if the pattern doesn't compile as a JS regex. */
function renderWithHighlight(
  line: string,
  pattern: string,
  ignoreCase: boolean,
): React.ReactNode {
  if (!pattern) return line
  let re: RegExp | null = null
  try {
    re = new RegExp(pattern, ignoreCase ? 'gi' : 'g')
  } catch {
    re = null
  }
  if (re) {
    const parts: React.ReactNode[] = []
    let last = 0
    let n = 0
    for (const hit of line.matchAll(re)) {
      const start = hit.index ?? 0
      const text = hit[0]
      // Skip zero-width matches that would otherwise loop forever.
      if (text.length === 0) continue
      if (start > last) parts.push(line.slice(last, start))
      parts.push(
        <span key={`m:${n}`} className="bg-accent/15 text-accent">
          {text}
        </span>,
      )
      last = start + text.length
      n++
      if (n > 200) break
    }
    if (last < line.length) parts.push(line.slice(last))
    return parts
  }
  // Substring fallback for patterns the JS regex engine rejects.
  const needle = ignoreCase ? pattern.toLowerCase() : pattern
  const hay = ignoreCase ? line.toLowerCase() : line
  const parts: React.ReactNode[] = []
  let i = 0
  let n = 0
  while (i < line.length) {
    const j = hay.indexOf(needle, i)
    if (j === -1) {
      parts.push(line.slice(i))
      break
    }
    if (j > i) parts.push(line.slice(i, j))
    parts.push(
      <span key={`s:${n++}`} className="bg-accent/15 text-accent">
        {line.slice(j, j + pattern.length)}
      </span>,
    )
    i = j + pattern.length
  }
  return parts
}
