import { Chip } from '@/components/chat/sandbox/terminal/Terminal'
import {
  fsRmRequestSchema,
  fsRmResponseSchema,
  safeParseResponse,
} from './parsers'
import { displayPath, isSandboxTarget, TargetChip } from './shared'

interface FsRmViewProps {
  input: unknown
  output: unknown
}

export function FsRmView({ input, output }: FsRmViewProps) {
  const req = fsRmRequestSchema.safeParse(input)
  if (!req.success) return null
  const resp = safeParseResponse(fsRmResponseSchema, output)
  if (!resp) return null
  const removed = resp.removed
  const sandboxed = isSandboxTarget(req.data.target)
  // `was_present` is a host-only signal; sandbox-target responses default
  // it (serde fill-in) — collapse to removed/not-removed wording there.
  const wasAbsent = !sandboxed && !removed && !resp.was_present
  const verb = removed
    ? '− removed '
    : wasAbsent
      ? '· was not present '
      : '· not removed '
  const tone = removed
    ? 'text-warn'
    : wasAbsent
      ? 'text-ink-ghost'
      : 'text-ink-faint'
  const recursive = req.data.recursive === true

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="px-3 py-3 flex flex-col gap-2">
        <div className="font-mono text-[12.5px] text-ink">
          <span className={tone}>{verb}</span>
          <span>{displayPath(req.data.path, resp.path)}</span>
        </div>
        {recursive || sandboxed ? (
          <div className="flex flex-wrap items-center gap-1.5">
            {recursive ? (
              <Chip label="recursive" className="border-warn text-warn">
                true
              </Chip>
            ) : null}
            <TargetChip target={req.data.target} />
          </div>
        ) : null}
      </div>
    </div>
  )
}
