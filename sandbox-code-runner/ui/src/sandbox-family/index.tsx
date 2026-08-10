/**
 * The sandbox::* family renderer — the console's first-party sandbox
 * chat family (console/web/src/components/chat/sandbox/) ported into
 * this worker as ONE injected `FunctionTriggerRenderer`, so the daemon
 * calls this worker's runtimes are built from render as cards wherever
 * the worker's UI is installed, console version notwithstanding.
 *
 * Dispatch mirrors the console exactly: the 15-id set is listed
 * explicitly (never derived from a prefix regex) so "is this a sandbox
 * call?" can never go too broad; error payloads are detected BEFORE the
 * per-op views (`parseSandboxErrorDisplay`), and every unparseable
 * payload falls through with `null` to the console's default JSON card.
 *
 * Unlike the worker's own three renderers (../function-trigger-message),
 * this family does NOT redact ids: a daemon `sandbox_id` is the
 * operator-visible address of a fleet-page row, not a capability like
 * `runtime_id` — deliberate design decision. No `redactRaw` here.
 */

import type { FunctionTriggerMessage, FunctionTriggerRenderer, Host } from '@iii-dev/console-ui'
import type { ReactNode } from 'react'
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
import { parseSandboxErrorDisplay, unwrapEnvelope } from './parsers'
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

function isSandboxFunction(functionId: string): boolean {
  return SANDBOX_FN_IDS.has(functionId)
}

/** The `sandbox::` prefix set faint so the op tail carries the line. */
export function SandboxFunctionIdLabel({ functionId }: { functionId: string }) {
  if (!functionId.startsWith('sandbox::')) {
    return <span className="cr-fam-fnid-tail">{functionId}</span>
  }
  const tail = functionId.slice('sandbox::'.length)
  return (
    <>
      <span className="cr-fam-fnid-prefix">sandbox::</span>
      <span className="cr-fam-fnid-tail">{tail}</span>
    </>
  )
}

function renderSettled(message: FunctionTriggerMessage, running: boolean): ReactNode | null {
  if (!isSandboxFunction(message.functionId)) return null
  if (message.pendingApproval) return null // tryRenderPreview handles it

  // The done-state view is what tryRender owns; the pending preview
  // lives in tryRenderPreview. Running-state cards are rendered by
  // the per-op view with `running=true` so the shell chrome stays
  // identical and only the body swaps to the executing-shimmer.
  const input = unwrapEnvelope(message.input)
  const rawOutput = message.output
  const output = rawOutput != null ? unwrapEnvelope(rawOutput) : undefined

  const errorDisplay = !running && rawOutput != null ? parseSandboxErrorDisplay(rawOutput) : null
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

function renderPreview(message: FunctionTriggerMessage): ReactNode | null {
  if (!message.pendingApproval) return null
  if (!isSandboxFunction(message.functionId)) return null
  const input = unwrapEnvelope(message.input)
  switch (message.functionId) {
    case 'sandbox::exec':
      return <ExecPreview input={input} />
    case 'sandbox::run':
      return <RunPreview input={input} />
    default:
      return null
  }
}

/** Renderers must never throw — a bad payload costs the card, never
    the feed. Anything unexpected falls through to the default view. */
function fenced(render: () => ReactNode | null): ReactNode | null {
  try {
    return render()
  } catch {
    return null
  }
}

export function createSandboxFamilyRenderer(host: Host): FunctionTriggerRenderer {
  void host // the family reads nothing off the host

  return {
    id: 'sandbox-code-runner/page.js#sandbox-family',
    isMatch: (functionId) => isSandboxFunction(functionId),
    tryRender: (message) => fenced(() => renderSettled(message, !!message.running)),
    tryRenderRunning: (message) => fenced(() => renderSettled(message, true)),
    tryRenderPreview: (message) => fenced(() => renderPreview(message)),
    FunctionIdLabel: SandboxFunctionIdLabel,
  }
}
