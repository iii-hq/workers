import { FooterPill, Terminal } from '../lib/terminal'
import { ShellExecChips, ShellExecPreviewRow } from './ExecView'
import { formatArgv, formatShellCommand } from './format'
import {
  type ShellExecBgRequest,
  type ShellExecBgResponse,
  safeParseResponse,
  shellExecBgRequestSchema,
  shellExecBgResponseSchema,
} from './parsers'
import { isSandboxTarget } from './shared'

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
    <Terminal
      command={formatShellCommand(req.data)}
      running={running}
      chips={<ShellExecChips req={req.data} bg />}
      footer={respData ? <ShellExecBgFooter req={req.data} /> : null}
    >
      {respData ? <BgStartedBody req={req.data} resp={respData} /> : null}
    </Terminal>
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
    <div className="shui-body">
      <div className="shui-baseline-row">
        <span className="t-accent">↻</span>
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
      <FooterPill tone="accent">job started</FooterPill>
      {/* Host-targeted background jobs IGNORE timeout_ms (wire semantic);
          surface the silently-dropped knob. */}
      {!isSandboxTarget(req.target) && typeof req.timeout_ms === 'number' ? (
        <FooterPill tone="warn">timeout_ms ignored (host bg)</FooterPill>
      ) : null}
    </>
  )
}
