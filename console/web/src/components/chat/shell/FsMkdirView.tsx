import { Chip } from '@/components/chat/sandbox/terminal/Terminal'
import {
  fsMkdirRequestSchema,
  fsMkdirResponseSchema,
  safeParseResponse,
} from './parsers'
import { displayPath, isSandboxTarget, TargetChip } from './shared'

interface FsMkdirViewProps {
  input: unknown
  output: unknown
}

export function FsMkdirView({ input, output }: FsMkdirViewProps) {
  const req = fsMkdirRequestSchema.safeParse(input)
  if (!req.success) return null
  const resp = safeParseResponse(fsMkdirResponseSchema, output)
  if (!resp) return null
  const created = resp.created
  const sandboxed = isSandboxTarget(req.data.target)
  // Sandbox-target responses default `already_existed` (serde fill-in,
  // not a signal) — collapse to the plain created/exists wording there.
  const verb = created
    ? '+ created '
    : sandboxed
      ? '· exists '
      : resp.already_existed
        ? '· already exists '
        : '· not created '

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="px-3 py-3 flex flex-col gap-2">
        <div className="font-mono text-[12.5px] text-ink">
          <span className={created ? 'text-accent' : 'text-ink-faint'}>
            {verb}
          </span>
          <span>{displayPath(req.data.path, resp.path)}</span>
        </div>
        <div className="flex flex-wrap items-center gap-1.5">
          <Chip label="mode">{req.data.mode ?? '0755'}</Chip>
          {req.data.parents ? <Chip label="parents">true</Chip> : null}
          <TargetChip target={req.data.target} />
        </div>
      </div>
    </div>
  )
}
