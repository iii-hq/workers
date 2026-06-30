import type { FunctionCallMessage } from '@/types/chat'
import { SubmitResultView } from './SubmitResultView'

/**
 * Synthetic harness tools — functions the harness injects into a turn rather
 * than ids owned by a worker. Currently just `submit_result` (the
 * output-contract fallback): its arguments are the turn's deliverable.
 */
export const HARNESS_FUNCTION_IDS = ['submit_result'] as const

export function isHarnessFunction(id: string): boolean {
  return id === 'submit_result'
}

/** Branded function-id label, mirroring the other namespace modules. */
export function HarnessFunctionIdLabel({ functionId }: { functionId: string }) {
  return <span className="text-ink font-medium">{functionId}</span>
}

function tryRender(message: FunctionCallMessage): React.ReactNode | null {
  if (!isHarnessFunction(message.functionId)) return null
  if (message.pendingApproval) return null
  return <SubmitResultView input={message.input} running={!!message.running} />
}

/** No bespoke pending preview; submit_result is never gated on approval. */
function tryRenderPreview(
  _message: FunctionCallMessage,
): React.ReactNode | null {
  return null
}

export const HarnessToolView = {
  isHarnessFunction,
  tryRender,
  /** Running state is handled inside `tryRender`. */
  tryRenderRunning: tryRender,
  tryRenderPreview,
}
