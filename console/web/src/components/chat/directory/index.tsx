import { SandboxErrorView } from '@/components/chat/sandbox/ErrorView'
import { parseSandboxErrorDisplay } from '@/components/chat/sandbox/parsers'
import type { FunctionCallMessage } from '@/types/chat'
import { SkillsDownloadView } from './DownloadView'
import { PromptsGetView, PromptsListView } from './PromptsViews'
import { isDirectoryFunction, unwrapEnvelope } from './parsers'
import {
  RegistryWorkerInfoView,
  RegistryWorkersListView,
} from './RegistryViews'
import { SkillsGetView, SkillsIndexView, SkillsListView } from './SkillsViews'

export function DirectoryFunctionIdLabel({
  functionId,
}: {
  functionId: string
}) {
  if (!functionId.startsWith('directory::')) {
    return <span className="text-ink">{functionId}</span>
  }
  const tail = functionId.slice('directory::'.length)
  return (
    <>
      <span className="text-ink-faint">directory::</span>
      <span className="text-ink font-medium">{tail}</span>
    </>
  )
}

function tryRender(message: FunctionCallMessage): React.ReactNode | null {
  if (!isDirectoryFunction(message.functionId)) return null
  if (message.pendingApproval) return null

  const input = unwrapEnvelope(message.input)
  const rawOutput = message.output
  const output = rawOutput != null ? unwrapEnvelope(rawOutput) : undefined
  const running = !!message.running

  // Reuse the sandbox error parser for the shared `function_error`
  // / gate-denial envelope — same translate layer.
  const errorDisplay =
    !running && rawOutput != null ? parseSandboxErrorDisplay(rawOutput) : null
  if (errorDisplay) {
    return <SandboxErrorView display={errorDisplay} />
  }

  switch (message.functionId) {
    case 'directory::skills::list':
      return <SkillsListView input={input} output={output} running={running} />
    case 'directory::skills::get':
      return <SkillsGetView input={input} output={output} running={running} />
    case 'directory::skills::index':
      return <SkillsIndexView input={input} output={output} running={running} />
    case 'directory::skills::download':
      return (
        <SkillsDownloadView input={input} output={output} running={running} />
      )
    case 'directory::prompts::list':
      return <PromptsListView input={input} output={output} running={running} />
    case 'directory::prompts::get':
      return <PromptsGetView input={input} output={output} running={running} />
    case 'directory::registry::workers::list':
      return (
        <RegistryWorkersListView
          input={input}
          output={output}
          running={running}
        />
      )
    case 'directory::registry::workers::info':
      return (
        <RegistryWorkerInfoView
          input={input}
          output={output}
          running={running}
        />
      )
    default:
      return null
  }
}

/**
 * Directory reads are non-destructive and don't go through the approval
 * gate. `download` could in principle (it touches disk), but the daemon
 * doesn't gate it today — returning `null` lets the default request JSON
 * pane handle the pending-state if it ever surfaces.
 */
function tryRenderPreview(
  _message: FunctionCallMessage,
): React.ReactNode | null {
  return null
}

export const DirectoryToolView = {
  isDirectoryFunction,
  tryRender,
  tryRenderRunning: tryRender,
  tryRenderPreview,
}
