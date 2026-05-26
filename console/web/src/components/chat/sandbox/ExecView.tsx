import { Prompt } from '@/components/ui/Prompt'
import { formatExecCommand, pillForExit, truncateMiddle } from './format'
import {
  type ExecRequest,
  type ExecResponse,
  execRequestSchema,
  execResponseSchema,
  safeParseResponse,
} from './parsers'
import { AnsiOutput } from './terminal/AnsiOutput'
import { Chip, FooterPill, Terminal } from './terminal/Terminal'

interface ExecViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

export function ExecView({ input, output, running }: ExecViewProps) {
  const req = execRequestSchema.safeParse(input)
  if (!req.success) return null
  const respData =
    output != null ? safeParseResponse(execResponseSchema, output) : null
  return (
    <Terminal
      command={formatExecCommand(req.data)}
      running={running}
      chips={<ExecChips req={req.data} />}
      footer={respData ? <ExecFooter resp={respData} /> : null}
    >
      <AnsiOutput stdout={respData?.stdout} stderr={respData?.stderr} />
    </Terminal>
  )
}

/** Compact `$ cmd args` preview used in the pending-approval state. */
export function ExecPreview({ input }: { input: unknown }) {
  const req = execRequestSchema.safeParse(input)
  if (!req.success) return null
  return (
    <div className="border-t border-rule-2 bg-paper-2 px-3 py-2 flex flex-wrap items-center gap-x-3 gap-y-1.5">
      <span className="font-mono text-[12.5px] text-ink whitespace-pre-wrap break-all">
        <Prompt symbol="$" />
        <span className="ml-2">{formatExecCommand(req.data)}</span>
      </span>
      <span className="flex flex-wrap items-center gap-1.5 ml-auto">
        <ExecChips req={req.data} />
      </span>
    </div>
  )
}

function ExecChips({ req }: { req: ExecRequest }) {
  return (
    <>
      <Chip label="sandbox">{truncateMiddle(req.sandbox_id, 12)}</Chip>
      {req.workdir ? <Chip label="cwd">{req.workdir}</Chip> : null}
      {typeof req.timeout_ms === 'number' ? (
        <Chip label="timeout">{`${req.timeout_ms}ms`}</Chip>
      ) : null}
    </>
  )
}

function ExecFooter({ resp }: { resp: ExecResponse }) {
  const exit = pillForExit(resp.exit_code)
  return (
    <>
      <FooterPill tone={exit.tone}>{exit.label}</FooterPill>
      <FooterPill>{`${resp.duration_ms}ms`}</FooterPill>
      {resp.timed_out ? <FooterPill tone="warn">timed out</FooterPill> : null}
      {/* 1.0 MiB cap matches `INLINE_BUFFER_CAP` in fs/read.rs; exec is
         buffered upstream with a comparable size, so flag when the
         payload looks like it hit the lid. */}
      {resp.stdout.length >= 1024 * 1024 ? (
        <FooterPill tone="warn">stdout cap (1.0 MiB) likely reached</FooterPill>
      ) : null}
    </>
  )
}
