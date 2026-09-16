import { Badge, Chip } from '@iii-dev/console-ui'
import { RefreshCw } from 'lucide-react'
import { execItems, ShellExecPreviewRow } from './ExecView'
import { formatArgv, formatShellCommand } from './format'
import {
  type ShellExecBgRequest,
  type ShellExecBgResponse,
  safeParseResponse,
  shellExecBgRequestSchema,
  shellExecBgResponseSchema,
} from './parsers'
import { isSandboxTarget, TerminalCard } from './shared'

interface ShellExecBgViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

export function ShellExecBgView({
  input,
  output,
  running,
}: ShellExecBgViewProps) {
  const req = shellExecBgRequestSchema.safeParse(input)
  if (!req.success) return null
  const respData =
    output != null ? safeParseResponse(shellExecBgResponseSchema, output) : null
  return (
    <TerminalCard
      command={formatShellCommand(req.data)}
      running={running}
      items={execItems(req.data)}
      chips={<Chip>bg</Chip>}
      footer={respData ? <ShellExecBgFooter req={req.data} /> : null}
    >
      {respData ? <BgStartedBody req={req.data} resp={respData} /> : null}
    </TerminalCard>
  )
}

/** Compact `$ cmd args` preview used in the pending-approval state —
    the shared exec row plus the `bg` marker chip. */
export function ShellExecBgPreview({ input }: { input: unknown }) {
  return <ShellExecPreviewRow input={input} bg />
}

/** Success body — there is no stdout for a background spawn. The job_id
    is the copy-handle for `shell::status`/`shell::kill`, so it renders
    full and untruncated. The server-resolved argv appears as a faint
    second line only when shell-words tokenization changed the spelling
    of what was typed (that's when it's informative). */
function BgStartedBody({
  req,
  resp,
}: {
  req: ShellExecBgRequest
  resp: ShellExecBgResponse
}) {
  const resolved = formatArgv(resp.argv)
  return (
    <div className="shui-card-body">
      <div className="shui-baseline-row">
        <RefreshCw aria-hidden className="shui-glyph t-accent" />
        <span className="t-ink">started</span>
        <code className="shui-inline-code">{resp.job_id}</code>
      </div>
      {resolved !== formatShellCommand(req) ? (
        <div className="shui-resolved-argv">{`$ ${resolved}`}</div>
      ) : null}
    </div>
  )
}

function ShellExecBgFooter({ req }: { req: ShellExecBgRequest }) {
  return (
    <>
      <Badge variant="accent">job started</Badge>
      {/* Host-targeted background jobs IGNORE timeout_ms (wire semantic);
          surface the silently-dropped knob. */}
      {!isSandboxTarget(req.target) && typeof req.timeout_ms === 'number' ? (
        <Badge variant="warn">timeout_ms ignored (host bg)</Badge>
      ) : null}
    </>
  )
}
