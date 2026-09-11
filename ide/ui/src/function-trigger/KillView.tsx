import { truncateMiddle } from '../lib/format'
import { ActionLine, FooterPill, StatusPill } from '../lib/terminal'
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
 * `shell::kill` — warn slab. The `killed: false` case always arrives
 * with `reason: "not running"` plus the job's terminal status; both
 * render side by side so the outcome is unambiguous.
 */
export function ShellKillView({ input, output, running }: ShellKillViewProps) {
  const req = shellKillRequestSchema.safeParse(input)
  if (!req.success) return null
  const resp =
    output != null ? safeParseResponse(shellKillResponseSchema, output) : null
  const status = resp ? jobStatusPill(resp.status) : null

  return (
    <div className="shui-card">
      <div className="shui-slab warn">
        <div className="shui-baseline-row">
          <span className="t-warn">×</span>
          <span className="t-ink">
            {running ? 'killing job…' : 'killed job'}
          </span>
          <code className="shui-inline-code">
            {truncateMiddle(resp?.job_id ?? req.data.job_id, 24)}
          </code>
        </div>
        {/* `reason` is long prose (e.g. the sandbox no-cancel-hook
           message) — full-width faint line, not a pill. */}
        {resp?.reason ? <div className="shui-note">{resp.reason}</div> : null}
        {resp && status ? (
          <div className="shui-row">
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
    <div className="shui-card">
      <ActionLine symbol="×" tone="warn">
        <span className="shui-line">
          kill job <code className="shui-inline-code">{req.data.job_id}</code>
        </span>
      </ActionLine>
    </div>
  )
}
