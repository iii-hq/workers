/**
 * Injected function-trigger renderer for the `shell::*` family — moved
 * out of the console SPA (formerly console/web/src/components/chat/shell/)
 * into the worker's own injected UI, so the rendering ships and versions
 * with the worker (the iii-directory precedent).
 *
 * Matches only the ids in `SHELL_FUNCTION_IDS`. Pending-approval
 * messages return null from `tryRender` so the console's approval bar
 * keeps handling them; the compact previews render through
 * `tryRenderPreview`. Error outputs are normalised by
 * `parseShellErrorDisplay` (S-code lifting + generic denial shapes) and
 * rendered by the shared error cards.
 */

import type {
  FunctionTriggerMessage,
  FunctionTriggerRenderer,
} from '@iii-dev/console-ui'
import { ErrorDisplayView } from '../lib/errors'
import { ShellConfigStatusView } from './ConfigStatusView'
import { ShellExecBgPreview, ShellExecBgView } from './ExecBgView'
import { ShellExecPreview, ShellExecView } from './ExecView'
import { FsSedView, FsGrepView } from './FsSearchViews'
import {
  FsChmodView,
  FsLsView,
  FsMkdirView,
  FsMvView,
  FsReadView,
  FsRmView,
  FsStatView,
} from './FsViews'
import { FsWriteView } from './FsWriteView'
import { ShellKillPreview, ShellKillView } from './KillView'
import { ShellListView } from './ListView'
import {
  isShellFunction,
  parseShellErrorDisplay,
  unwrapEnvelope,
} from './parsers'
import { ShellStatusView } from './StatusView'

/** Renders outside the script's scope wrapper (the card header), so it
    uses inline token styles instead of scoped classes. */
function ShellFunctionIdLabel({ functionId }: { functionId: string }) {
  if (!functionId.startsWith('shell::')) {
    return <span style={{ color: 'var(--color-ink)' }}>{functionId}</span>
  }
  const tail = functionId.slice('shell::'.length)
  return (
    <>
      <span style={{ color: 'var(--color-ink-faint)' }}>shell::</span>
      <span style={{ color: 'var(--color-ink)', fontWeight: 500 }}>{tail}</span>
    </>
  )
}

function render(message: FunctionTriggerMessage): React.ReactNode | null {
  if (!isShellFunction(message.functionId)) return null
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
    !running && rawOutput != null ? parseShellErrorDisplay(rawOutput) : null
  if (errorDisplay) {
    return <ErrorDisplayView display={errorDisplay} />
  }

  switch (message.functionId) {
    case 'shell::exec':
      return <ShellExecView input={input} output={output} running={running} />
    case 'shell::exec_bg':
      return <ShellExecBgView input={input} output={output} running={running} />
    case 'shell::status':
      return <ShellStatusView input={input} output={output} running={running} />
    case 'shell::kill':
      return <ShellKillView input={input} output={output} running={running} />
    case 'shell::list':
      return <ShellListView output={output} />
    case 'shell::config-status':
      return <ShellConfigStatusView output={output} running={running} />
    case 'shell::fs::ls':
      return <FsLsView input={input} output={output} />
    case 'shell::fs::stat':
      return <FsStatView input={input} output={output} />
    case 'shell::fs::read':
      return <FsReadView input={input} output={output} />
    case 'shell::fs::write':
      return <FsWriteView input={input} output={output} />
    case 'shell::fs::mkdir':
      return <FsMkdirView input={input} output={output} />
    case 'shell::fs::rm':
      return <FsRmView input={input} output={output} />
    case 'shell::fs::mv':
      return <FsMvView input={input} output={output} />
    case 'shell::fs::chmod':
      return <FsChmodView input={input} output={output} />
    case 'shell::fs::grep':
      return <FsGrepView input={input} output={output} />
    case 'shell::fs::sed':
      return <FsSedView input={input} output={output} />
    default:
      return null
  }
}

function tryRenderPreview(
  message: FunctionTriggerMessage,
): React.ReactNode | null {
  if (!isShellFunction(message.functionId)) return null
  const input = unwrapEnvelope(message.input)
  switch (message.functionId) {
    case 'shell::exec':
      return <ShellExecPreview input={input} />
    case 'shell::exec_bg':
      return <ShellExecBgPreview input={input} />
    case 'shell::kill':
      return <ShellKillPreview input={input} />
    default:
      return null
  }
}

export function createShellTriggerRenderer(): FunctionTriggerRenderer {
  return {
    id: 'shell/page.js#shell',
    isMatch: isShellFunction,
    tryRender: render,
    /** Alias kept deliberately; running state lives inside `render`. */
    tryRenderRunning: render,
    tryRenderPreview,
    FunctionIdLabel: ShellFunctionIdLabel,
  }
}
