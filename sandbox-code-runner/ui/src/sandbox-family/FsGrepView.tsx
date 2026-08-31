/** `sandbox::fs::grep` — highlighted match list (patterns are regexes). */

import { renderWithHighlight } from './highlight'
import { type FsMatch, fsGrepRequestSchema, fsGrepResponseSchema, safeParseResponse } from './parsers'
import { Chip, FooterPill, SandboxIdChip } from './shared'

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
    <div className="cr-fam-card">
      <div className="cr-fam-chips-row">
        <SandboxIdChip sandboxId={req.data.sandbox_id} />
        <Chip label="path">{req.data.path}</Chip>
        <Chip label="pattern">{req.data.pattern}</Chip>
        {req.data.ignore_case ? <Chip>case-insensitive</Chip> : null}
        <FooterPill tone={matches.length > 0 ? 'default' : 'warn'}>
          {`${matches.length} ${matches.length === 1 ? 'match' : 'matches'}`}
        </FooterPill>
        {truncated ? <FooterPill tone="warn">truncated</FooterPill> : null}
      </div>

      {matches.length === 0 ? (
        <div className="cr-fam-note-ghost">· no matches</div>
      ) : (
        <GrepMatchList matches={matches} pattern={req.data.pattern} ignoreCase={!!req.data.ignore_case} />
      )}
    </div>
  )
}

interface GrepMatchListProps {
  matches: FsMatch[]
  pattern: string
  ignoreCase: boolean
}

/** Highlighted match list — the wire speaks `FsMatch` and
    regex-by-default patterns. */
function GrepMatchList({ matches, pattern, ignoreCase }: GrepMatchListProps) {
  return (
    <div className="cr-fam-grep">
      {matches.map((m) => (
        <div
          /* path+line+content is collision-resistant in practice (two
             hits on the same line of the same file produce identical
             FsMatch records on the wire today). React will only warn
             if the daemon ever sends true duplicates, which would
             itself be a wire-shape bug worth surfacing. */
          key={`${m.path}:${m.line}:${m.content}`}
          className="cr-fam-grep-item"
        >
          <div className="cr-fam-grep-loc">
            <span className="cr-fam-grep-path">{m.path}</span>
            <span className="ghost">:</span>
            <span className="num">{m.line}</span>
          </div>
          <pre className="cr-fam-grep-line">
            <code>
              {/* sandbox grep patterns are always regexes on the wire */}
              {renderWithHighlight(m.content, pattern, {
                isRegex: true,
                ignoreCase,
              })}
            </code>
          </pre>
        </div>
      ))}
    </div>
  )
}
