import { AnsiOutput } from '@/components/chat/sandbox/terminal/AnsiOutput'
import { Badge } from '@/components/ui/Badge'
import {
  execResponseSchema,
  type SandboxDispatchDenial,
  type SandboxErrorDisplay,
  type SandboxErrorWire,
  type SandboxInvocationError,
  safeParseResponse,
} from './parsers'

interface ErrorViewProps {
  error: SandboxErrorWire
}

interface InvocationErrorViewProps {
  error: SandboxInvocationError
}

interface DispatchDeniedViewProps {
  denial: SandboxDispatchDenial
}

interface SandboxErrorViewProps {
  display: SandboxErrorDisplay
}

/**
 * Visualises the flat `{ type, code, message, docs_url, retryable,
 * fix, fix_note }` shape produced by `SandboxError::to_payload`. The
 * card is a warn-toned slab so it stands out from successful tool
 * cards without being as loud as the alert-red used for transport
 * errors. `S200` (exec timeout) deliberately leaks through here
 * rather than into `ExecView`: it carries no `ExecResponse`, so the
 * terminal chrome would be misleading.
 */
function execStreamsFromFix(error: SandboxErrorWire) {
  if (error.code !== 'S200' || error.fix == null) return null
  const parsed = safeParseResponse(execResponseSchema, error.fix)
  if (!parsed) return null
  const { stdout, stderr } = parsed
  if (!stdout && !stderr) return null
  return { stdout, stderr }
}

export function ErrorView({ error }: ErrorViewProps) {
  const retryable = error.retryable === true
  const streams = execStreamsFromFix(error)
  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="px-3 py-3 flex flex-col gap-2 border-l-2 border-warn">
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant="warn" className="border border-warn px-1.5 py-0.5">
            {error.code}
          </Badge>
          <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
            {error.type}
          </span>
          {retryable ? (
            <Badge variant="accent" className="normal-case tracking-normal">
              retryable
            </Badge>
          ) : null}
        </div>

        <pre className="font-mono text-[12.5px] leading-[1.55] text-ink whitespace-pre-wrap break-words m-0">
          <code>{error.message}</code>
        </pre>

        {error.fix_note ? (
          <div className="font-mono text-[12px] leading-[1.6] text-ink-faint italic">
            {error.fix_note}
          </div>
        ) : null}

        {error.docs_url ? (
          <a
            href={error.docs_url}
            target="_blank"
            rel="noreferrer noopener"
            className="font-mono text-[11px] uppercase tracking-[0.06em] text-accent hover:underline w-fit"
          >
            docs ↗
          </a>
        ) : null}

        {streams ? (
          <div className="mt-1 border border-rule-2 rounded-sm overflow-hidden">
            <AnsiOutput stdout={streams.stdout} stderr={streams.stderr} />
          </div>
        ) : null}
      </div>
    </div>
  )
}

export function InvocationErrorView({ error }: InvocationErrorViewProps) {
  const badge = error.deniedBy ?? 'error'
  const showDetailText =
    error.detailText &&
    error.detailText !== error.message &&
    error.detailText !== error.reason

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="px-3 py-3 flex flex-col gap-2 border-l-2 border-warn">
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant="warn" className="border border-warn px-1.5 py-0.5">
            {badge}
          </Badge>
          <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
            {error.title}
          </span>
        </div>

        {error.functionId ? (
          <div className="font-mono text-[12px] leading-[1.6] text-ink-faint">
            <span className="uppercase tracking-[0.06em] text-[11px]">
              function
            </span>{' '}
            <code className="text-ink">{error.functionId}</code>
          </div>
        ) : null}

        <pre className="font-mono text-[12.5px] leading-[1.55] text-ink whitespace-pre-wrap break-words m-0">
          <code>{error.message}</code>
        </pre>

        {showDetailText ? (
          <pre className="font-mono text-[12px] leading-[1.6] text-ink-faint whitespace-pre-wrap break-words m-0">
            <code>{error.detailText}</code>
          </pre>
        ) : null}
      </div>
    </div>
  )
}

/**
 * A structural dispatch-policy denial. Unlike a generic invocation failure,
 * the function is outside a legacy positive scope or matched by a deny glob.
 * The card names the blocked id and points to the policy locations that can
 * narrow a workflow node or session.
 */
export function DispatchDeniedView({ denial }: DispatchDeniedViewProps) {
  const fn = denial.functionId
  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="px-3 py-3 flex flex-col gap-2 border-l-2 border-warn">
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant="warn" className="border border-warn px-1.5 py-0.5">
            denied
          </Badge>
          <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
            dispatch policy
          </span>
        </div>

        {fn ? (
          <div className="font-mono text-[12px] leading-[1.6] text-ink-faint">
            <span className="uppercase tracking-[0.06em] text-[11px]">
              blocked
            </span>{' '}
            <code className="text-ink">{fn}</code>
          </div>
        ) : null}

        <div className="font-mono text-[12.5px] leading-[1.6] text-ink">
          {fn ? (
            <>
              This agent's function policy blocks <code>{fn}</code>. Adjust it
              where the agent is defined:
            </>
          ) : (
            <>
              This function is blocked by the agent's policy. Adjust it where
              the agent is defined:
            </>
          )}
          <ul className="mt-1.5 flex flex-col gap-1 text-[12px] text-ink-faint">
            <li>
              <span className="text-ink">workflow node</span> — its{' '}
              <code className="text-ink">agent.functions</code> (or the def's{' '}
              <code className="text-ink">default_functions</code>) narrows it.
              Remove the matching deny or widen a legacy allow scope.
            </li>
            <li>
              <span className="text-ink">chat / session</span> — remove the
              matching <code className="text-ink">options.functions.deny</code>{' '}
              glob, or widen a legacy{' '}
              <code className="text-ink">options.functions.allow</code>.
            </li>
          </ul>
        </div>

        <pre className="font-mono text-[11.5px] leading-[1.55] text-ink-faint whitespace-pre-wrap break-words m-0">
          <code>{denial.message}</code>
        </pre>
      </div>
    </div>
  )
}

export function SandboxErrorView({ display }: SandboxErrorViewProps) {
  if (display.variant === 'wire') {
    return <ErrorView error={display.error} />
  }
  if (display.variant === 'dispatch-denied') {
    return <DispatchDeniedView denial={display.error} />
  }
  return <InvocationErrorView error={display.error} />
}
