import { renderWithHighlight } from './highlight'
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
                  {/* sandbox grep patterns are always regexes on the wire */}
                  {renderWithHighlight(m.content, req.data.pattern, {
                    isRegex: true,
                    ignoreCase: !!req.data.ignore_case,
                  })}
                </code>
              </pre>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
