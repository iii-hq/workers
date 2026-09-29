import { Badge, Chip, type MetaRowItem } from '@iii-dev/console-ui'
import { formatBytes, formatDuration } from '@iii-dev/console-ui/format'
import { pillForExit, truncateMiddle } from '../lib/format'
import { formatShellCommand } from './format'
import {
  type ShellExecRequest,
  type ShellExecResponse,
  safeParseResponse,
  shellExecRequestSchema,
  shellExecResponseSchema,
} from './parsers'
import { items, kv, StreamBody, targetItem, TerminalCard } from './shared'

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
    <TerminalCard
      command={formatShellCommand(req.data)}
      running={running}
      items={execItems(req.data)}
      footer={respData ? <ShellExecFooter resp={respData} /> : null}
    >
      <StreamBody stdout={respData?.stdout} stderr={respData?.stderr} />
    </TerminalCard>
  )
}

/** Compact `$ cmd args` preview used in the pending-approval state. */
export function ShellExecPreview({ input }: { input: unknown }) {
  return <ShellExecPreviewRow input={input} />
}

/** Shared approval-surface row for exec and exec_bg previews (exec_bg
    adds the `bg` marker chip). The items carry the approval-relevant
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
    <TerminalCard
      command={formatShellCommand(req.data)}
      items={execItems(req.data)}
      chips={bg ? <Chip>bg</Chip> : null}
    />
  )
}

/* `stdin.length` undercounts multibyte input — the server caps and pipes
   bytes, so report the UTF-8 size. */
const utf8 = new TextEncoder()

/** Header items shared by exec/exec_bg cards and both previews. `env`
    surfaces sorted KEYS only — values can be secrets (the raw-json tab
    still has them). */
export function execItems(req: ShellExecRequest): MetaRowItem[] {
  const envKeys = req.env ? Object.keys(req.env).sort() : []
  return items(
    targetItem(req.target, true),
    req.cwd ? kv('cwd', truncateMiddle(req.cwd, 28)) : null,
    typeof req.timeout_ms === 'number' ? kv('timeout', `${req.timeout_ms}ms`) : null,
    envKeys.length > 0 ? kv('env', truncateMiddle(envKeys.join(', '), 24)) : null,
    req.stdin != null ? kv('stdin', formatBytes(utf8.encode(req.stdin).length)) : null,
  )
}

function ShellExecFooter({ resp }: { resp: ShellExecResponse }) {
  const exit = pillForExit(resp.exit_code)
  return (
    <>
      <Badge variant={exit.tone}>{exit.label}</Badge>
      <Badge>{formatDuration(resp.duration_ms)}</Badge>
      {resp.timed_out ? <Badge variant="warn">timed out</Badge> : null}
      {/* Explicit wire flags — no length heuristic; the server says so. */}
      {resp.stdout_truncated ? <Badge variant="warn">stdout truncated</Badge> : null}
      {resp.stderr_truncated ? <Badge variant="warn">stderr truncated</Badge> : null}
    </>
  )
}
