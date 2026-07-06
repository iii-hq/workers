/**
 * `coder::context` — compact workspace card: primary root, platform,
 * git branch + dirty count (or "not a repository"), and instruction
 * file names with their content behind a disclosure. Discovery data
 * like InfoView, but about the WORKSPACE rather than the worker config.
 */
import { Chip, FooterPill } from '@/components/chat/sandbox/terminal/Terminal'
import {
  type ContextGit,
  contextRequestSchema,
  contextResponseSchema,
  type InstructionFile,
  safeParseRequest,
  safeParseResponse,
} from './parsers'

interface ContextViewProps {
  input: unknown
  output?: unknown
  running?: boolean
  preview?: boolean
}

/** "clean" / "3 dirty" / "50+ dirty" — status lines ARE the dirty
    entries; truncation means the count is a floor, not exact. */
export function dirtyLabel(status: string[], truncated: boolean): string {
  if (status.length === 0 && !truncated) return 'clean'
  return `${status.length}${truncated ? '+' : ''} dirty`
}

function GitRow({ git }: { git: ContextGit | null | undefined }) {
  return (
    <div className="border-b border-rule-2 last:border-b-0 px-3 py-2 flex flex-wrap items-center gap-1.5">
      {git ? (
        <>
          <Chip label="branch">{git.branch}</Chip>
          <FooterPill tone={git.status.length > 0 ? 'warn' : 'accent'}>
            {dirtyLabel(git.status, git.status_truncated)}
          </FooterPill>
        </>
      ) : (
        <span className="font-mono text-[12px] text-ink-ghost">
          · not a repository
        </span>
      )}
    </div>
  )
}

function InstructionFileRow({ file }: { file: InstructionFile }) {
  return (
    <details className="border-b border-rule-2 last:border-b-0">
      <summary className="px-3 py-1.5 flex flex-wrap items-center gap-2 cursor-pointer list-none select-none">
        <span className="font-mono text-[12px] text-ink break-all">
          {file.path}
        </span>
        {file.truncated ? <FooterPill tone="warn">truncated</FooterPill> : null}
      </summary>
      <pre className="px-3 pb-2 font-mono text-[12px] leading-[1.55] text-ink whitespace-pre-wrap break-all">
        {file.content}
      </pre>
    </details>
  )
}

export function ContextView({
  input,
  output,
  running,
  preview,
}: ContextViewProps) {
  // Request is `{}` — trivially satisfiable, only bail on non-object junk.
  const req = safeParseRequest(contextRequestSchema, input)
  if (!req) return null
  const resp =
    output != null && !preview
      ? safeParseResponse(contextResponseSchema, output)
      : null

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="bg-paper-2 border-b border-rule-2 px-3 py-2 flex flex-wrap items-center gap-1.5">
        <span className="font-mono text-[12.5px] text-ink">
          <span className="text-ink-faint">coder </span>
          <span>context</span>
        </span>
        {resp ? (
          <Chip label="platform">
            {resp.platform.os}/{resp.platform.arch}
          </Chip>
        ) : null}
        {running ? (
          <span className="font-mono text-[11px] text-ink-ghost animate-pulse">
            · querying…
          </span>
        ) : null}
      </div>

      {resp ? (
        <>
          <div className="border-b border-rule-2 px-3 py-2 flex flex-wrap items-center gap-1.5">
            <Chip label="root">{resp.primary_root}</Chip>
          </div>
          <GitRow git={resp.git} />
          {resp.instruction_files.map((file) => (
            <InstructionFileRow key={file.path} file={file} />
          ))}
        </>
      ) : running ? null : (
        <div className="px-3 py-2 font-mono text-[12px] text-ink-ghost">
          · no workspace reported
        </div>
      )}
    </div>
  )
}
