/**
 * Injected function-trigger renderer for the `directory::*` family —
 * moved out of the console SPA (formerly
 * console/web/src/components/chat/directory/) into the worker's own
 * injected UI, so the rendering ships and versions with the worker.
 *
 * Matches only the ids in `DIRECTORY_FUNCTION_IDS`. Error outputs and
 * pending-approval messages return null so the console's default cards
 * (error chips, approval bar, JSON panes) keep handling them — injected
 * renderers should stay narrow.
 */

import type {
  FunctionTriggerMessage,
  FunctionTriggerRenderer,
} from '@iii-dev/console-ui'
import { isErrorOutput, unwrapEnvelope } from '../lib/envelope'
import { AgentsGetView, AgentsListView, AgentsUpdateView } from './AgentsViews'
import { SkillsDownloadView } from './DownloadView'
import { isDirectoryFunction } from './parsers'
import {
  SystemPromptsGetView,
  SystemPromptsListView,
} from './SystemPromptsViews'
import {
  RegistryWorkerInfoView,
  RegistryWorkersListView,
} from './RegistryViews'
import { SkillsGetView, SkillsIndexView, SkillsListView } from './SkillsViews'
import { SkillsUpdateView, SystemPromptsUpdateView } from './UpdateViews'

function FunctionIdLabel({ functionId }: { functionId: string }) {
  if (!functionId.startsWith('directory::')) {
    return <span style={{ color: 'var(--color-ink)' }}>{functionId}</span>
  }
  const tail = functionId.slice('directory::'.length)
  return (
    <>
      <span style={{ color: 'var(--color-ink-faint)' }}>directory::</span>
      <span style={{ color: 'var(--color-ink)', fontWeight: 500 }}>
        {tail}
      </span>
    </>
  )
}

function render(
  message: FunctionTriggerMessage,
  running: boolean,
): React.ReactNode | null {
  if (!isDirectoryFunction(message.functionId)) return null
  if (message.pendingApproval) return null

  const input = unwrapEnvelope(message.input)
  // Error outputs fall through to the console's default error cards.
  if (!running && message.output != null && isErrorOutput(message.output)) {
    return null
  }
  const output =
    message.output != null ? unwrapEnvelope(message.output) : undefined

  switch (message.functionId) {
    case 'directory::skills::list':
      return <SkillsListView input={input} output={output} running={running} />
    case 'directory::skills::get':
      return <SkillsGetView input={input} output={output} running={running} />
    case 'directory::skills::index':
      return <SkillsIndexView input={input} output={output} running={running} />
    case 'directory::skills::download':
    case 'directory::skills::download_from_registry':
    case 'directory::skills::download_from_repo':
      return (
        <SkillsDownloadView input={input} output={output} running={running} />
      )
    case 'directory::skills::update':
      return (
        <SkillsUpdateView input={input} output={output} running={running} />
      )
    /* create's wire shapes ({id, content} → {id, title, bytes, modified_at})
       are identical to update's, so the update view fits. */
    case 'directory::skills::create':
      return (
        <SkillsUpdateView
          input={input}
          output={output}
          running={running}
          verb="created"
        />
      )
    case 'directory::system-prompts::list':
      return (
        <SystemPromptsListView
          input={input}
          output={output}
          running={running}
        />
      )
    case 'directory::system-prompts::get':
      return (
        <SystemPromptsGetView input={input} output={output} running={running} />
      )
    case 'directory::system-prompts::update':
      return (
        <SystemPromptsUpdateView
          input={input}
          output={output}
          running={running}
        />
      )
    /* create's wire shapes ({name, content} → {name, description, bytes,
       modified_at}) are identical to update's, so the update view fits. */
    case 'directory::system-prompts::create':
      return (
        <SystemPromptsUpdateView
          input={input}
          output={output}
          running={running}
          verb="created"
        />
      )
    case 'directory::agents::list':
      return <AgentsListView input={input} output={output} running={running} />
    case 'directory::agents::get':
      return <AgentsGetView input={input} output={output} running={running} />
    case 'directory::agents::update':
      return (
        <AgentsUpdateView input={input} output={output} running={running} />
      )
    /* create's wire shapes ({id, content} → {id, name, description, logo,
       bytes, modified_at}) are identical to update's, so the update view fits. */
    case 'directory::agents::create':
      return (
        <AgentsUpdateView
          input={input}
          output={output}
          running={running}
          verb="created"
        />
      )
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

export function createDirectoryTriggerRenderer(): FunctionTriggerRenderer {
  return {
    id: 'iii-directory/page.js#directory',
    isMatch: isDirectoryFunction,
    tryRender: (message) => render(message, !!message.running),
    tryRenderRunning: (message) => render(message, true),
    // Directory reads aren't approval-gated; the writes keep the default
    // request-JSON preview pane when a gate ever fronts them.
    tryRenderPreview: () => null,
    FunctionIdLabel,
  }
}
