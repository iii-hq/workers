import { formatBytes, truncateMiddle, pillForExit } from '../lib/format'
import { AnsiOutput, Chip, FooterPill, Prompt, Terminal } from '../lib/terminal'
import { formatShellCommand } from './format'
import {
  type ShellExecRequest,
  type ShellExecResponse,
  safeParseResponse,
  shellExecRequestSchema,
  shellExecResponseSchema,
} from './parsers'
import { TargetChip } from './shared'

interface ShellExecViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

export function ShellExecView({ input, output, running }: ShellExecViewProps) {
  const req = shellExecRequestSchema.safeParse(input)
  if (!req.success) return null
  const respData =
    output != null ? safeParseResponse(shellExecResponseSchema, output) : null
  return (
    <Terminal
      command={formatShellCommand(req.data)}
      running={running}
      chips={<ShellExecChips req={req.data} />}
      footer={respData ? <ShellExecFooter resp={respData} /> : null}
    >
      <AnsiOutput stdout={respData?.stdout} stderr={respData?.stderr} />
    </Terminal>
  )
}

/** Compact `$ cmd args` preview used in the pending-approval state. */
export function ShellExecPreview({ input }: { input: unknown }) {
  return <ShellExecPreviewRow input={input} />
}

/** Shared approval-surface row for exec and exec_bg previews (exec_bg
    adds the `bg` marker chip). The chips carry the approval-relevant
    facts: target (host is the privileged one), cwd, env keys, stdin
    size. */
export function ShellExecPreviewRow({
  input,
  bg,
}: {
  input: unknown
  bg?: boolean
}) {
  const req = shellExecRequestSchema.safeParse(input)
  if (!req.success) return null
  return (
    <div className="shui-card shui-head preview">
      <span className="shui-cmd">
        <Prompt symbol="$" />
        <span className="cmd-text">{formatShellCommand(req.data)}</span>
      </span>
      <span className="shui-chips end">
        <ShellExecChips req={req.data} bg={bg} />
      </span>
    </div>
  )
}

/* `stdin.length` undercounts multibyte input — the server caps and pipes
   bytes, so report the UTF-8 size. */
const utf8 = new TextEncoder()

/** Header chips shared by exec/exec_bg cards and both previews. `env`
    surfaces sorted KEYS only — values can be secrets (the raw-json tab
    still has them). */
export function ShellExecChips({
  req,
  bg,
}: {
  req: ShellExecRequest
  bg?: boolean
}) {
  const envKeys = req.env ? Object.keys(req.env).sort() : []
  return (
    <>
      {bg ? <Chip>bg</Chip> : null}
      <TargetChip target={req.target} explicitHost />
      {req.cwd ? <Chip label="cwd">{truncateMiddle(req.cwd, 28)}</Chip> : null}
      {typeof req.timeout_ms === 'number' ? (
        <Chip label="timeout">{`${req.timeout_ms}ms`}</Chip>
      ) : null}
      {envKeys.length > 0 ? (
        <Chip label="env">{truncateMiddle(envKeys.join(', '), 24)}</Chip>
      ) : null}
      {req.stdin != null ? (
        <Chip label="stdin">{formatBytes(utf8.encode(req.stdin).length)}</Chip>
      ) : null}
    </>
  )
}

function ShellExecFooter({ resp }: { resp: ShellExecResponse }) {
  const exit = pillForExit(resp.exit_code)
  return (
    <>
      <FooterPill tone={exit.tone}>{exit.label}</FooterPill>
      <FooterPill>{`${resp.duration_ms}ms`}</FooterPill>
      {resp.timed_out ? <FooterPill tone="warn">timed out</FooterPill> : null}
      {/* Explicit wire flags — no length heuristic; the server says so. */}
      {resp.stdout_truncated ? (
        <FooterPill tone="warn">stdout truncated</FooterPill>
      ) : null}
      {resp.stderr_truncated ? (
        <FooterPill tone="warn">stderr truncated</FooterPill>
      ) : null}
    </>
  )
}
