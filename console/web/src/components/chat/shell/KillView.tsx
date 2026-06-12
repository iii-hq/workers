import { truncateMiddle } from '@/components/chat/sandbox/format'
import { ActionLine, StatusPill } from '@/components/chat/sandbox/shared'
import { FooterPill } from '@/components/chat/sandbox/terminal/Terminal'
import { jobStatusPill } from './format'
import {
  safeParseResponse,
  shellKillRequestSchema,
  shellKillResponseSchema,
} from './parsers'

interface ShellKillViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

/**
 * `shell::kill` — warn slab in the sandbox StopView pattern. The
 * `killed: false` case always arrives with `reason: "not running"`
 * plus the job's terminal status; both render side by side so the
 * outcome is unambiguous.
 */
export function ShellKillView({ input, output, running }: ShellKillViewProps) {
  const req = shellKillRequestSchema.safeParse(input)
  if (!req.success) return null
  const resp =
    output != null ? safeParseResponse(shellKillResponseSchema, output) : null
  const status = resp ? jobStatusPill(resp.status) : null

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="px-3 py-3 flex flex-col gap-2 border-l-2 border-warn">
        <div className="font-mono text-[12.5px] text-ink flex flex-wrap items-baseline gap-2">
          <span className="text-warn">×</span>
          <span>{running ? 'killing job…' : 'killed job'}</span>
          <code className="bg-paper-2 border border-rule-2 px-1.5 py-0.5 text-[12px] text-ink">
            {truncateMiddle(resp?.job_id ?? req.data.job_id, 24)}
          </code>
        </div>
        {/* `reason` is long prose (e.g. the sandbox no-cancel-hook
           message) — full-width faint line, not a pill. */}
        {resp?.reason ? (
          <div className="font-mono text-[12px] leading-[1.6] text-ink-faint italic">
            {resp.reason}
          </div>
        ) : null}
        {resp && status ? (
          <div className="flex flex-wrap items-center gap-1.5">
            <StatusPill label={status.label} variant={status.tone} />
            <FooterPill tone={resp.killed ? 'accent' : 'warn'}>
              {resp.killed ? 'killed' : 'not running'}
            </FooterPill>
          </div>
        ) : null}
      </div>
    </div>
  )
}

/** Pending-approval preview — kill is destructive, so the approver
    sees the exact job id on a single warn action line. */
export function ShellKillPreview({ input }: { input: unknown }) {
  const req = shellKillRequestSchema.safeParse(input)
  if (!req.success) return null
  return (
    <div className="border-t border-rule-2 bg-bg">
      <ActionLine symbol="×" tone="warn">
        <span className="font-mono text-[12.5px]">
          kill job{' '}
          <code className="bg-paper-2 border border-rule-2 px-1.5 py-0.5 text-[12px]">
            {req.data.job_id}
          </code>
        </span>
      </ActionLine>
    </div>
  )
}
