/**
 * Injected function-call renderer for the `workflow::*` family — moved out
 * of the console SPA (formerly ade/web/src/components/chat/workflow/) so the
 * cards ship and version with the worker.
 *
 * Matches the ids in `WORKFLOW_FUNCTION_IDS`. Error outputs and pending
 * approvals return null so the console's default cards (error chips,
 * approval bar, JSON panes) keep handling them; so do the internal
 * lifecycle ids (tick/sweep/wake), which only get the branded label.
 */

import type { FunctionCallMessage, FunctionCallRenderer, Host } from '@iii-dev/console-ui'
import type { ReactNode } from 'react'
import { isWorkflowFunction, safeParseRequest, startRequestSchema, unwrapEnvelope } from '../lib/parsers'
import { NodeResultView } from './NodeResultView'
import { StartView } from './StartView'
import { StatusView } from './StatusView'
import { StopView } from './StopView'

function FunctionIdLabel({ functionId }: { functionId: string }) {
  return (
    <>
      <span style={{ color: 'var(--color-ink-faint)' }}>workflow::</span>
      <span style={{ color: 'var(--color-ink)', fontWeight: 500 }}>{functionId.slice('workflow::'.length)}</span>
    </>
  )
}

/** Error-shaped output (engine `{ error: ... }` wrapper). */
function isErrorOutput(value: unknown): boolean {
  return !!value && typeof value === 'object' && !Array.isArray(value) && 'error' in value
}

function tryRender(message: FunctionCallMessage): ReactNode | null {
  if (!isWorkflowFunction(message.functionId)) return null
  if (message.pendingApproval) return null

  const running = !!message.running
  if (!running && message.output != null && isErrorOutput(message.output)) return null

  const input = unwrapEnvelope(message.input)
  const output = message.output != null ? unwrapEnvelope(message.output) : undefined

  switch (message.functionId) {
    case 'workflow::start': {
      // A request that does not parse (an old-shape start) gets the card's
      // own request and response panes instead of an empty custom view.
      const req = safeParseRequest(startRequestSchema, input)
      return req ? <StartView req={req} output={output} running={running} /> : null
    }
    case 'workflow::status':
      return <StatusView input={input} output={output} running={running} />
    case 'workflow::node-result':
      return <NodeResultView input={input} output={output} running={running} />
    case 'workflow::stop':
      return <StopView input={input} output={output} running={running} />
    default:
      return null
  }
}

export function createWorkflowRenderer(): FunctionCallRenderer {
  return {
    id: 'workflow/page.js#workflow',
    isMatch: isWorkflowFunction,
    tryRender,
    // Running state lives inside `tryRender` (`message.running`).
    tryRenderRunning: tryRender,
    FunctionIdLabel,
  }
}

export function registerWorkflowRenderer(host: Host): () => void {
  return host.functionCalls.register(createWorkflowRenderer())
}
