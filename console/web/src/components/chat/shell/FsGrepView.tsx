import { GrepMatchList } from '@/components/chat/sandbox/FsGrepView'
import { truncateMiddle } from '@/components/chat/sandbox/format'
import { Chip, FooterPill } from '@/components/chat/sandbox/terminal/Terminal'
import {
  fsGrepRequestSchema,
  fsGrepResponseSchema,
  safeParseResponse,
} from './parsers'
import { TargetChip } from './shared'

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
        <TargetChip target={req.data.target} />
        {req.data.ignore_case ? <Chip>case-insensitive</Chip> : null}
        {/* `recursive` defaults TRUE on the wire — chip the deviation only */}
        {req.data.recursive === false ? <Chip>non-recursive</Chip> : null}
        {req.data.include_glob?.length ? (
          <Chip label="include">
            {truncateMiddle(req.data.include_glob.join(' '), 40)}
          </Chip>
        ) : null}
        {req.data.exclude_glob?.length ? (
          <Chip label="exclude">
            {truncateMiddle(req.data.exclude_glob.join(' '), 40)}
          </Chip>
        ) : null}
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
        <GrepMatchList
          matches={matches}
          pattern={req.data.pattern}
          ignoreCase={!!req.data.ignore_case}
        />
      )}
    </div>
  )
}
