import { AnsiOutput } from '@/components/chat/sandbox/terminal/AnsiOutput'
import { ErrorCard, type ErrorCardProps } from './ErrorCard'
import {
  execResponseSchema,
  type SandboxDispatchDenial,
  type SandboxErrorDisplay,
  type SandboxErrorWire,
  type SandboxInvocationError,
  safeParseResponse,
} from './parsers'

interface SandboxErrorViewProps {
  display: SandboxErrorDisplay
}

/**
 * Normalizes every supported sandbox/invocation failure into one shared card.
 * Parsing remains in parsers.ts; this adapter owns only user-facing language
 * and presentation metadata.
 */
export function SandboxErrorView({ display }: SandboxErrorViewProps) {
  const props = errorCardProps(display)
  return (
    <ErrorCard {...props} />
  )
}

function errorCardProps(display: SandboxErrorDisplay): ErrorCardProps {
  switch (display.variant) {
    case 'wire':
      return wireErrorCardProps(display.error)
    case 'dispatch-denied':
      return dispatchDeniedCardProps(display.error)
    case 'invocation':
      return invocationErrorCardProps(display.error)
  }
}

function wireErrorCardProps(error: SandboxErrorWire): ErrorCardProps {
  const streams = execStreamsFromFix(error)
  return {
    badge: error.code,
    title: error.code === 'S200' ? 'Command timed out' : 'Tool failed',
    category: humanize(error.type),
    message: error.message,
    retryable: error.retryable === true,
    metadata: [{ label: 'Type', value: <code>{error.type}</code> }],
    ...(error.fix_note
      ? {
          guidance: <p>{error.fix_note}</p>,
        }
      : {}),
    ...(error.docs_url ? { docsUrl: error.docs_url } : {}),
    ...(streams
      ? {
          output: (
            <AnsiOutput stdout={streams.stdout} stderr={streams.stderr} />
          ),
        }
      : {}),
  }
}

function invocationErrorCardProps(
  error: SandboxInvocationError,
): ErrorCardProps {
  const showDetailText =
    error.detailText &&
    error.detailText !== error.message &&
    error.detailText !== error.reason
  const metadata: ErrorCardProps['metadata'] = [
    ...(error.functionId
      ? [{ label: 'Function', value: <code>{error.functionId}</code> }]
      : []),
    ...(error.deniedBy
      ? [{ label: 'Denied by', value: <code>{error.deniedBy}</code> }]
      : []),
  ]

  return {
    badge: error.deniedBy ? 'Denied' : 'Error',
    title: error.title,
    category: 'Invocation error',
    message: error.message,
    metadata,
    ...(showDetailText ? { technicalDetails: error.detailText } : {}),
  }
}

function dispatchDeniedCardProps(
  denial: SandboxDispatchDenial,
): ErrorCardProps {
  const metadata: ErrorCardProps['metadata'] = [
    ...(denial.functionId
      ? [
          {
            label: 'Blocked function',
            value: <code>{denial.functionId}</code>,
          },
        ]
      : []),
    ...(denial.namespace
      ? [{ label: 'Namespace', value: <code>{denial.namespace}</code> }]
      : []),
  ]

  return {
    badge: 'Denied',
    title: 'Function blocked by policy',
    category: 'Dispatch policy',
    message: denial.functionId
      ? 'This function is outside the agent’s dispatch allow-list.'
      : 'The requested function is outside the agent’s dispatch allow-list.',
    metadata,
    guidanceTitle: 'How to resolve',
    guidance: (
      <div className="flex flex-col gap-2">
        <p>Update the scope where this agent is defined:</p>
        <ul className="list-disc space-y-1.5 pl-5">
          <li>
            <span className="font-medium text-ink">Workflow node</span> — add
            the function to <code className="text-ink">agent.functions</code>,
            widen <code className="text-ink">default_functions</code>, or remove
            the narrowing to inherit the run’s allowed functions
          </li>
          <li>
            <span className="font-medium text-ink">Chat or session</span> — add
            it to <code className="text-ink">options.functions.allow</code>
          </li>
        </ul>
      </div>
    ),
    technicalDetails: denial.message,
  }
}

/** S200 carries a partial ExecResponse in `fix`; other fixes are not streams. */
function execStreamsFromFix(error: SandboxErrorWire) {
  if (error.code !== 'S200' || error.fix == null) return null
  const parsed = safeParseResponse(execResponseSchema, error.fix)
  if (!parsed) return null
  const { stdout, stderr } = parsed
  if (!stdout && !stderr) return null
  return { stdout, stderr }
}

function humanize(value: string): string {
  const words = value.replace(/[_-]+/g, ' ').trim()
  if (!words) return 'Sandbox error'
  return words.charAt(0).toUpperCase() + words.slice(1)
}
