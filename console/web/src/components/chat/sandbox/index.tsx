import type { FunctionCallMessage } from '@/types/chat'
import { CreateView } from './CreateView'
import { SandboxErrorView } from './ErrorView'
import { ExecPreview, ExecView } from './ExecView'
import { FsChmodView } from './FsChmodView'
import { FsGrepView } from './FsGrepView'
import { FsLsView } from './FsLsView'
import { FsMkdirView } from './FsMkdirView'
import { FsMvView } from './FsMvView'
import { FsReadView } from './FsReadView'
import { FsRmView } from './FsRmView'
import { FsSedView } from './FsSedView'
import { FsStatView } from './FsStatView'
import { FsWriteView } from './FsWriteView'
import { ListView } from './ListView'
import {
  execRequestSchema,
  parseSandboxErrorDisplay,
  runRequestSchema,
  unwrapEnvelope,
} from './parsers'
import { RunPreview, RunView } from './RunView'
import { StopView } from './StopView'

/* The known sandbox::* set. Listed explicitly (not derived from
   regex) so the dispatcher's "is this a sandbox call?" check is
   never accidentally too broad. */
const SANDBOX_FN_IDS = new Set([
  'sandbox::exec',
  'sandbox::run',
  'sandbox::create',
  'sandbox::stop',
  'sandbox::list',
  'sandbox::fs::ls',
  'sandbox::fs::stat',
  'sandbox::fs::read',
  'sandbox::fs::write',
  'sandbox::fs::mkdir',
  'sandbox::fs::rm',
  'sandbox::fs::mv',
  'sandbox::fs::chmod',
  'sandbox::fs::grep',
  'sandbox::fs::sed',
])

export function isSandboxFunction(functionId: string): boolean {
  return SANDBOX_FN_IDS.has(functionId)
}

/* Public surface mirrors the plan exactly. Both helpers return `null`
   for unknown function ids or unparseable payloads so the caller can
   silently fall back to the existing JSON view. */
export function SandboxFunctionIdLabel({ functionId }: { functionId: string }) {
  if (!functionId.startsWith('sandbox::')) {
    return <span className="text-ink">{functionId}</span>
  }
  const tail = functionId.slice('sandbox::'.length)
  return (
    <>
      <span className="text-ink-faint">sandbox::</span>
      <span className="text-ink font-medium">{tail}</span>
    </>
  )
}

function tryRender(message: FunctionCallMessage): React.ReactNode | null {
  if (!isSandboxFunction(message.functionId)) return null
  if (message.pendingApproval) return null

  // The done-state view is what tryRender owns; the pending preview
  // lives in tryRenderPreview. Running-state cards are rendered by
  // the per-tool view with `running=true` so the shell chrome stays
  // identical and only the body swaps to the executing-shimmer.
  const input = unwrapEnvelope(message.input)
  const rawOutput = message.output
  const output = rawOutput != null ? unwrapEnvelope(rawOutput) : undefined
  const running = !!message.running

  const errorDisplay =
    !running && rawOutput != null ? parseSandboxErrorDisplay(rawOutput) : null
  if (errorDisplay) {
    return <SandboxErrorView display={errorDisplay} />
  }

  switch (message.functionId) {
    case 'sandbox::exec':
      return <ExecView input={input} output={output} running={running} />
    case 'sandbox::run':
      return <RunView input={input} output={output} running={running} />
    case 'sandbox::create':
      return <CreateView input={input} output={output} running={running} />
    case 'sandbox::stop':
      return <StopView input={input} output={output} running={running} />
    case 'sandbox::list':
      return <ListView output={output} />
    case 'sandbox::fs::ls':
      return <FsLsView input={input} output={output} />
    case 'sandbox::fs::stat':
      return <FsStatView input={input} output={output} />
    case 'sandbox::fs::read':
      return <FsReadView input={input} output={output} />
    case 'sandbox::fs::write':
      return <FsWriteView input={input} output={output} />
    case 'sandbox::fs::mkdir':
      return <FsMkdirView input={input} output={output} />
    case 'sandbox::fs::rm':
      return <FsRmView input={input} output={output} />
    case 'sandbox::fs::mv':
      return <FsMvView input={input} output={output} />
    case 'sandbox::fs::chmod':
      return <FsChmodView input={input} output={output} />
    case 'sandbox::fs::grep':
      return <FsGrepView input={input} output={output} />
    case 'sandbox::fs::sed':
      return <FsSedView input={input} output={output} />
    default:
      return null
  }
}

function tryRenderPreview(
  message: FunctionCallMessage,
): React.ReactNode | null {
  if (!isSandboxFunction(message.functionId)) return null
  const input = unwrapEnvelope(message.input)
  // Parse-check BEFORE building an element: the preview components render
  // null for an unparseable request, but a non-null element here makes
  // FunctionCallCard suppress its raw request pane (the null contract).
  switch (message.functionId) {
    case 'sandbox::exec':
      return execRequestSchema.safeParse(input).success ? (
        <ExecPreview input={input} />
      ) : null
    case 'sandbox::run':
      return runRequestSchema.safeParse(input).success ? (
        <RunPreview input={input} />
      ) : null
    default:
      return null
  }
}

export const SandboxToolView = {
  isSandboxFunction,
  tryRender,
  /** Alias kept for FCM symmetry; running state lives inside `tryRender`. */
  tryRenderRunning: tryRender,
  tryRenderPreview,
}
