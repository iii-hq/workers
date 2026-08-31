import { SandboxErrorView } from '@/components/chat/sandbox/ErrorView'
import { parseSandboxErrorDisplay } from '@/components/chat/sandbox/parsers'
import type { FunctionTriggerMessage } from '@/types/chat'
import { isHarnessFunction, unwrapEnvelope } from './parsers'
import { SpawnActivityDisplay, SpawnPreview, SpawnView } from './SpawnView'
import { SubmitResultView } from './SubmitResultView'

/**
 * Harness tool family — synthetic tools the harness injects into a turn
 * (`submit_result`, the output-contract fallback) and harness-owned ids
 * (`harness::spawn`, the sub-agent pending trigger).
 */
export { HARNESS_FUNCTION_IDS, isHarnessFunction } from './parsers'

/** Branded function-id label, mirroring the other namespace modules. */
export function HarnessFunctionIdLabel({ functionId }: { functionId: string }) {
  if (!functionId.startsWith('harness::')) {
    return <span className="text-ink font-medium">{functionId}</span>
  }
  const tail = functionId.slice('harness::'.length)
  return (
    <>
      <span className="text-ink-faint">harness::</span>
      <span className="text-ink font-medium">{tail}</span>
    </>
  )
}

function tryRender(message: FunctionTriggerMessage): React.ReactNode | null {
  if (!isHarnessFunction(message.functionId)) return null
  if (message.pendingApproval) return null

  switch (message.functionId) {
    case 'submit_result':
      return (
        <SubmitResultView input={message.input} running={!!message.running} />
      )
    case 'harness::spawn':
      return tryRenderSpawn(message)
    default:
      return null
  }
}

function tryRenderSpawn(
  message: FunctionTriggerMessage,
): React.ReactNode | null {
  if (message.functionId !== 'harness::spawn' || message.pendingApproval)
    return null
  const running = !!message.running
  const rawOutput = message.output
  // Guard errors (spawn depth/fan-out), failed/cancelled children and
  // gate denials all arrive as error envelopes — surface them before
  // success parsing, mirroring web/index.tsx.
  const errorDisplay =
    !running && rawOutput != null ? parseSandboxErrorDisplay(rawOutput) : null
  if (errorDisplay) return <SandboxErrorView display={errorDisplay} />
  return (
    <SpawnView
      input={unwrapEnvelope(message.input)}
      // the ONE unwrap — SpawnView must never unwrap again
      output={rawOutput != null ? unwrapEnvelope(rawOutput) : undefined}
      running={running}
    />
  )
}

function tryRenderSpawnDisplay(
  message: FunctionTriggerMessage,
): React.ReactNode | null {
  if (
    message.functionId !== 'harness::spawn' ||
    message.pendingApproval ||
    message.running ||
    parseSandboxErrorDisplay(message.output)
  ) {
    return null
  }
  return (
    <SpawnActivityDisplay
      input={unwrapEnvelope(message.input)}
      output={
        message.output != null ? unwrapEnvelope(message.output) : undefined
      }
      parentSessionId={message.sessionId}
      functionTriggerId={message.functionTriggerId}
      createdAt={message.createdAt}
    />
  )
}

/** `submit_result` is never gated on approval; spawn is. */
function tryRenderPreview(
  message: FunctionTriggerMessage,
): React.ReactNode | null {
  if (message.functionId !== 'harness::spawn') return null
  return <SpawnPreview input={unwrapEnvelope(message.input)} />
}

export const HarnessToolView = {
  isHarnessFunction,
  tryRender,
  /** Running state is handled inside the views. */
  tryRenderRunning: tryRender,
  tryRenderPreview,
}

/** Focused entry registered ahead of the harness family so only spawn gets
 * prominent chat metadata and its compact live child-session surface. */
export const HarnessSpawnToolView = {
  isHarnessSpawnFunction: (functionId: string) =>
    functionId === 'harness::spawn',
  tryRender: tryRenderSpawn,
  tryRenderRunning: tryRenderSpawn,
  tryRenderDisplay: tryRenderSpawnDisplay,
}
