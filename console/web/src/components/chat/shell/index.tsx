import { SandboxErrorView } from '@/components/chat/sandbox/ErrorView'
import type { FunctionCallMessage } from '@/types/chat'
import { ShellConfigStatusView } from './ConfigStatusView'
import { ShellExecBgPreview, ShellExecBgView } from './ExecBgView'
import { ShellExecPreview, ShellExecView } from './ExecView'
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
import { ShellKillPreview, ShellKillView } from './KillView'
import { ShellListView } from './ListView'
import {
  isShellFunction,
  parseShellErrorDisplay,
  unwrapEnvelope,
} from './parsers'
import { ShellStatusView } from './StatusView'

/* The known shell::* set lives in parsers.ts (SHELL_FUNCTION_IDS) so
   the dispatcher and the schemas share one source of truth. Re-exported
   here for FCM symmetry with the sandbox module. */
export { isShellFunction, SHELL_FUNCTION_IDS } from './parsers'

/* Public surface mirrors the sandbox module exactly. Both helpers
   return `null` for unknown function ids or unparseable payloads so
   the caller can silently fall back to the existing JSON view. */
export function ShellFunctionIdLabel({ functionId }: { functionId: string }) {
  if (!functionId.startsWith('shell::')) {
    return <span className="text-ink">{functionId}</span>
  }
  const tail = functionId.slice('shell::'.length)
  return (
    <>
      <span className="text-ink-faint">shell::</span>
      <span className="text-ink font-medium">{tail}</span>
    </>
  )
}

function tryRender(message: FunctionCallMessage): React.ReactNode | null {
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
    return <SandboxErrorView display={errorDisplay} />
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
  message: FunctionCallMessage,
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

export const ShellToolView = {
  isShellFunction,
  tryRender,
  /** Alias kept for FCM symmetry; running state lives inside `tryRender`. */
  tryRenderRunning: tryRender,
  tryRenderPreview,
}
