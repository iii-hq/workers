/**
 * Any worker's agent run, rendered as the child session it created.
 *
 * The sub-agent card used to belong to one function id (`harness::spawn`), so
 * a run started on any other agent worker — `claude::start`,
 * `run::start_and_wait`, a worker nobody has written yet — read as a bare JSON
 * blob even though it had produced a real child session the console already
 * knew about. Nothing about the card is harness-shaped, so nothing about it
 * needs a function id: what identifies an agent run is its RESPONSE, the
 * shared agent entrypoint's shape.
 *
 * A run is recognised when the response carries a session id plus one of the
 * fields that only an agent run answers with:
 *
 * - `child_session_id` — the spawn acknowledgement
 * - `session_id` + `started` — a run accepted and left running
 * - `session_id` + `result` / `usage` / `turn_id` — a run that finished
 *
 * `session_id` alone is deliberately not enough: plenty of functions hand back
 * a session of some other kind (a PTY session, for one), and claiming those
 * would put a sub-agent card on a terminal.
 *
 * This renderer sits LAST in the first-party order, so every family that knows
 * its own ids still wins; it only ever claims what nothing else did.
 */

import { useRelativeClock } from '@/hooks/use-relative-clock'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import type { FunctionTriggerMessage } from '@/types/chat'
import { unwrapEnvelope } from '../harness/parsers'
import {
  SpawnActivityCard,
  useLiveSubagentActivity,
} from '../harness/SpawnView'
import { displayedSubagentActivity } from '../harness/subagent-activity'

/** The response of one agent run, as far as this card needs to read it. */
type AgentRunResponse = {
  sessionId: string
  /** The run is still going: no result yet, and it said so. */
  started: boolean
}

function str(value: unknown): string | undefined {
  return typeof value === 'string' && value.length > 0 ? value : undefined
}

/**
 * The child session an agent-run response names, or null when the response is
 * not one. Reads the response only — the input cannot be trusted for this: a
 * caller may pass `session_id` to a function that never starts a run.
 */
export function agentRunResponse(output: unknown): AgentRunResponse | null {
  if (!output || typeof output !== 'object') return null
  const record = unwrapEnvelope(output) as Record<string, unknown>
  if (!record || typeof record !== 'object') return null
  const child = str(record.child_session_id)
  if (child) return { sessionId: child, started: record.result === undefined }
  const sessionId = str(record.session_id)
  if (!sessionId) return null
  if (record.started === true) return { sessionId, started: true }
  const finished =
    record.result !== undefined ||
    record.usage !== undefined ||
    'turn_id' in record
  return finished ? { sessionId, started: false } : null
}

/** What was asked, from whichever field this worker's request uses. */
export function requestedTask(input: unknown): string | null {
  if (input == null) return null
  const record = unwrapEnvelope(input) as Record<string, unknown>
  if (!record || typeof record !== 'object') return null
  const direct = str(record.prompt) ?? str(record.task)
  if (direct) return direct
  const messages = Array.isArray(record.messages) ? record.messages : []
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index] as Record<string, unknown> | null
    if (message?.role !== 'user') continue
    const content = message.content
    if (typeof content === 'string') return content
    if (!Array.isArray(content)) continue
    const text = content
      .map((block) =>
        block && typeof block === 'object'
          ? str((block as Record<string, unknown>).text)
          : undefined,
      )
      .filter(Boolean)
      .join('\n')
    if (text) return text
  }
  return null
}

interface AgentSessionDisplayProps {
  input: unknown
  output: unknown
  createdAt?: number
}

/**
 * The live child-session card, on the session the run reported. The card is
 * the same one a spawned harness child gets: a delegated run is a delegated
 * run whoever answers it.
 */
export function AgentSessionDisplay({
  input,
  output,
  createdAt,
}: AgentSessionDisplayProps) {
  const run = agentRunResponse(output)
  const ctx = useConversationsCtxOptional()
  const sessionId = run?.sessionId ?? null
  const child = ctx?.conversations.find(
    (conversation) => conversation.id === sessionId,
  )
  const signal = useLiveSubagentActivity(sessionId, child?.status)
  const activity = displayedSubagentActivity(
    child,
    signal,
    ctx?.connectionState,
  )
  const clock = useRelativeClock(
    child?.createdAt ?? createdAt ?? signal?.timestamp ?? child?.updatedAt,
  )
  if (!sessionId) return null

  const task = requestedTask(input) ?? 'Waiting for the assigned task.'
  const title =
    child?.subagentAppearance?.name?.trim() ||
    (child?.title && child.title !== child.id ? child.title : task)
  const open =
    ctx && sessionId
      ? () => {
          ctx.openConversationInPanel(sessionId)
        }
      : undefined

  return (
    <SpawnActivityCard
      title={title}
      task={task}
      status={activity}
      sessionId={sessionId}
      icon={child?.subagentAppearance?.icon ?? 'agent'}
      color={child?.subagentAppearance?.color ?? 'neutral'}
      createdAt={child?.createdAt ?? createdAt}
      activityAt={
        activity === 'disconnected'
          ? undefined
          : (signal?.timestamp ?? child?.updatedAt ?? child?.createdAt)
      }
      now={clock}
      onOpen={open}
    />
  )
}

/**
 * Claimed from the MESSAGE, never from the function id — the decision has to
 * be synchronous, because a renderer that returns an element which later
 * renders `null` still counts as a claim and would hide the JSON panes.
 */
function tryRender(message: FunctionTriggerMessage): React.ReactNode | null {
  if (message.pendingApproval || message.running) return null
  if (!agentRunResponse(message.output)) return null
  return (
    <div className="border-t border-rule-2 bg-bg p-3">
      <AgentSessionDisplay
        input={message.input}
        output={message.output}
        createdAt={message.createdAt}
      />
    </div>
  )
}

export const AgentSessionToolView = {
  /** Every id: the response decides, and `tryRender` is where it decides. */
  isMatch: () => true,
  tryRender,
}
