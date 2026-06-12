import { Chip, FooterPill } from '@/components/chat/sandbox/terminal/Terminal'
import {
  fsMvRequestSchema,
  fsMvResponseSchema,
  safeParseResponse,
} from './parsers'
import { displayPath, isSandboxTarget, TargetChip } from './shared'

interface FsMvViewProps {
  input: unknown
  output: unknown
}

export function FsMvView({ input, output }: FsMvViewProps) {
  const req = fsMvRequestSchema.safeParse(input)
  if (!req.success) return null
  const resp = safeParseResponse(fsMvResponseSchema, output)
  if (!resp) return null
  const moved = resp.moved
  /* Sandbox-target responses default `overwrote` — a serde fill-in, not
     a signal; only surface the warn pill for host moves. */
  const showOverwrote = resp.overwrote && !isSandboxTarget(req.data.target)
  const hasChips =
    isSandboxTarget(req.data.target) || !!req.data.overwrite || showOverwrote

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="px-3 py-3 flex flex-col gap-2">
        <div className="font-mono text-[12.5px] flex flex-wrap items-baseline gap-2 text-ink">
          <span className={moved ? 'text-accent' : 'text-ink-faint'}>
            {moved ? 'mv' : '·'}
          </span>
          <span>{displayPath(req.data.src, resp.src)}</span>
          <span className="text-ink-ghost">→</span>
          <span>{displayPath(req.data.dst, resp.dst)}</span>
        </div>
        {hasChips ? (
          <div className="flex flex-wrap items-center gap-1.5">
            <TargetChip target={req.data.target} />
            {req.data.overwrite ? <Chip label="overwrite">true</Chip> : null}
            {showOverwrote ? (
              <FooterPill tone="warn">overwrote existing</FooterPill>
            ) : null}
          </div>
        ) : null}
      </div>
    </div>
  )
}
