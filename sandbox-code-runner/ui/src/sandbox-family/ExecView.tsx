/**
 * `sandbox::exec` — the terminal card. Ported from the console's
 * ExecView with the family upgrades: a single exit-reason verdict pill
 * (timed-out / not-found / not-executable folded in), SGR-aware
 * streams, the 1 MiB truncation chip (inside `Streams`), and a
 * sandbox-id chip that copies on click and jumps to the fleet page.
 */

import { TerminalCommandLine } from '@iii-dev/console-ui'
import { exitReason, formatExecCommand } from './format'
import {
  type ExecRequest,
  type ExecResponse,
  execRequestSchema,
  execResponseSchema,
  safeParseResponse,
} from './parsers'
import { Badge } from '@iii-dev/console-ui'
import { Chip, ExitReasonPill, SandboxIdChip, Streams, Terminal } from './shared'

interface ExecViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

export function ExecView({ input, output, running }: ExecViewProps) {
  const req = execRequestSchema.safeParse(input)
  if (!req.success) return null
  const respData = output != null ? safeParseResponse(execResponseSchema, output) : null
  return (
    <Terminal
      command={formatExecCommand(req.data)}
      running={running}
      chips={<ExecChips req={req.data} />}
      footer={respData ? <ExecFooter resp={respData} /> : null}
    >
      <Streams stdout={respData?.stdout} stderr={respData?.stderr} />
    </Terminal>
  )
}

/** Compact `$ cmd args` preview used in the pending-approval state. */
export function ExecPreview({ input }: { input: unknown }) {
  const req = execRequestSchema.safeParse(input)
  if (!req.success) return null
  return (
    <div className="cr-fam-card">
      <TerminalCommandLine
        command={formatExecCommand(req.data)}
        chips={<ExecChips req={req.data} />}
        className="cr-fam-term-head"
      />
    </div>
  )
}

function ExecChips({ req }: { req: ExecRequest }) {
  return (
    <>
      <SandboxIdChip sandboxId={req.sandbox_id} />
      {req.workdir ? <Chip label="cwd">{req.workdir}</Chip> : null}
      {typeof req.timeout_ms === 'number' ? <Chip label="timeout">{`${req.timeout_ms}ms`}</Chip> : null}
    </>
  )
}

function ExecFooter({ resp }: { resp: ExecResponse }) {
  return (
    <>
      <ExitReasonPill reason={exitReason(resp)} />
      <Badge>{`${resp.duration_ms}ms`}</Badge>
    </>
  )
}
