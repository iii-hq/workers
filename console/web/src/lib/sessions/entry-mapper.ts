/**
 * Maps session-manager transcript entries to the console's UI message model
 * and reconciles live event snapshots into an ordered `Message[]`.
 *
 * Identity scheme (the reconciliation contract):
 * - A single-segment entry (user message, custom entry) renders as one UI
 *   message whose id IS the `entry_id`. The optimistic user message the
 *   console appends on send uses the predicted entry id
 *   (`e_idem_<message_id>`, derived from the `idempotency_key` the harness
 *   uses to seed the user entry), so the `message-added` snapshot replaces it
 *   in place.
 * - An assistant entry splits per content block into segments with ids
 *   `<entry_id>:<block_index>` (thinking → thought, text → assistant,
 *   function_call → function-call row). Each `message-updated` snapshot
 *   re-derives the segment list wholesale and replaces the entry's range.
 * - `function_result` entries render no row of their own; they fill the
 *   `output` of the function-call row with the matching `functionCallId`.
 * - `custom_type: "compaction"` custom entries render the compaction marker.
 *
 * Events are at-least-once and unordered: callers keep the highest
 * `revision` per entry (see use-conversations) and treat `session::messages`
 * read-backs as the source of truth.
 */

import { parseAttachedFileHeader } from '@/lib/file-mentions'
import type {
  Attachment,
  FunctionCallMessage,
  Message,
  SystemMessage,
  TriggerFiredData,
  UserMessage,
} from '@/types/chat'
import type { AgentMessage, ContentBlock, TranscriptItem } from './types'

export const COMPACTION_CUSTOM_TYPE = 'compaction'
export const TRIGGER_FIRED_CUSTOM_TYPE = 'trigger_fired'
/** Harness turn-failure record (`{ reason }`) — `finalize_failed`. */
export const ERROR_CUSTOM_TYPE = 'error'
/** Harness informational record (`{ reason, message }`) — e.g. max_turns. */
export const NOTICE_CUSTOM_TYPE = 'notice'

/**
 * Map one typed custom record (however it arrived — `item.custom` on events,
 * a `role: 'custom'` message on `session::messages` read-backs) to its UI
 * segments. `null` means "not a typed record" so the caller can fall back.
 */
function customSegments(
  entryId: string,
  customType: string,
  data: unknown,
  timestamp: number,
): Message[] | null {
  switch (customType) {
    case COMPACTION_CUSTOM_TYPE:
      return [compactionMarker(entryId, data, timestamp)]
    case TRIGGER_FIRED_CUSTOM_TYPE:
      return [triggerFiredMessage(entryId, data, timestamp)]
    // Harness failure/notice records: without these the turn can end in an
    // error the chat never shows (the session flips to "error" silently).
    case ERROR_CUSTOM_TYPE:
      return [customNotice(entryId, data, 'error', 'turn failed', timestamp)]
    case NOTICE_CUSTOM_TYPE:
      return [customNotice(entryId, data, 'info', 'notice', timestamp)]
    default:
      return null
  }
}

function customNotice(
  entryId: string,
  data: unknown,
  tone: 'info' | 'error',
  fallback: string,
  timestamp: number,
): SystemMessage {
  const d = (data ?? {}) as { reason?: unknown; message?: unknown }
  const content =
    typeof d.message === 'string'
      ? d.message
      : typeof d.reason === 'string'
        ? tone === 'error'
          ? `turn failed — ${d.reason}`
          : d.reason
        : fallback
  return {
    id: entryId,
    role: 'system',
    kind: 'notice',
    content,
    tone,
    createdAt: timestamp,
  }
}

/** The trigger's display name: label, else join id, else state scope/key. */
export function triggerFiredName(t: TriggerFiredData): string {
  if (t.label) return t.label
  if (t.join) return `join ${t.join.id}`
  if (t.key) return t.scope ? `${t.scope}/${t.key}` : t.key
  return 'trigger'
}

/** Plain one-liner for the fired notice (also the a11y/fallback content). */
export function triggerFiredSummary(t: TriggerFiredData): string {
  const name = triggerFiredName(t)
  if (t.join && !t.join.completed) {
    return `${name} · ${t.join.arrived}/${t.join.expected} arrived`
  }
  const action =
    t.target === 'spawn'
      ? `spawned${t.model ? ` ${t.model}` : ''}`
      : 'notified this chat'
  return `${name} · ${action}${t.retired ? ' · unregistered' : ''}`
}

function triggerFiredMessage(
  entryId: string,
  data: unknown,
  timestamp: number,
): SystemMessage {
  const t = (data ?? {}) as TriggerFiredData
  return {
    id: entryId,
    role: 'system',
    kind: 'trigger-fired',
    content: triggerFiredSummary(t),
    tone: 'info',
    trigger: t,
    createdAt: typeof t.fired_at === 'number' ? t.fired_at : timestamp,
  }
}

/** The harness wraps every tool call in agent_trigger; unwrap for display. */
function unwrapFunctionCall(
  block: Extract<ContentBlock, { type: 'function_call' }>,
): {
  functionId: string
  input: unknown
  unresolvedTarget?: boolean
} {
  if (block.function_id === 'agent_trigger') {
    if (block.arguments && typeof block.arguments === 'object') {
      const args = block.arguments as {
        function?: unknown
        payload?: unknown
        _streaming?: unknown
      }
      // Mid-stream, the harness rides the raw in-flight arguments tail on
      // `_streaming` (providers degrade the incomplete JSON itself), so the
      // command can be watched forming in the request pane.
      const streaming =
        typeof args._streaming === 'string' ? args._streaming : undefined
      if (typeof args.function === 'string' && args.function.length > 0) {
        if (streaming !== undefined) {
          return { functionId: args.function, input: { _streaming: streaming } }
        }
        return { functionId: args.function, input: args.payload ?? {} }
      }
      if (streaming !== undefined) {
        return {
          functionId: block.function_id,
          input: { _streaming: streaming },
          unresolvedTarget: true,
        }
      }
    }
    // The target is unknown while the wrapper's arguments are still
    // streaming (providers degrade partial JSON to `{}`). Flag it so the UI
    // can render a placeholder instead of the literal `agent_trigger`; the
    // next snapshot self-corrects once the arguments finish streaming.
    return {
      functionId: block.function_id,
      input: block.arguments,
      unresolvedTarget: true,
    }
  }
  return { functionId: block.function_id, input: block.arguments }
}

function textOf(blocks: ContentBlock[]): string {
  let out = ''
  for (const block of blocks) {
    if (block.type === 'text') out += block.text
  }
  return out
}

/**
 * Split a user message's blocks into visible text and attachment chips.
 * `<attached-file …>` blocks are console-authored `#file(...)` mention
 * expansions — rendering their full content in the user bubble would dump
 * whole files into the chat, so they collapse to chips instead (failure
 * placeholders keep the error visible in the chip name).
 */
function splitUserContent(blocks: ContentBlock[]): {
  text: string
  attachments: Attachment[]
} {
  let text = ''
  const attachments: Attachment[] = []
  for (const block of blocks) {
    if (block.type !== 'text') continue
    const header = parseAttachedFileHeader(block.text)
    if (header) {
      attachments.push({
        id: `mention-${header.path}`,
        name: header.error ? `${header.path} (${header.error})` : header.path,
        size: header.size ?? 0,
        type: 'text/x-file-mention',
      })
    } else {
      text += block.text
    }
  }
  return { text, attachments }
}

/**
 * `harness::react` appends the firing event (or a join's gathered inputs) to
 * the task as a trailing `<event>`/`<inputs>` fenced-JSON block. Split it off
 * so the task renders as clean prose and the payload as collapsible JSON.
 * Tolerant of whitespace collapse; pretty-prints when the JSON parses.
 */
const REACTION_APPENDIX =
  /\n*<(event|inputs)>\s*(?:```json\n?)?([\s\S]*?)(?:\n?```)?\s*<\/\1>\s*$/

export function splitReactionTask(content: string): {
  task: string
  appendix?: { label: 'event' | 'inputs'; json: string }
} {
  const m = content.match(REACTION_APPENDIX)
  if (!m || m.index === undefined) return { task: content }
  let json = m[2].trim()
  try {
    json = JSON.stringify(JSON.parse(json), null, 2)
  } catch {
    // Not valid JSON (truncated event?) — show it raw rather than hide it.
  }
  return {
    task: content.slice(0, m.index).trimEnd(),
    appendix: { label: m[1] as 'event' | 'inputs', json },
  }
}

function compactionMarker(
  entryId: string,
  data: unknown,
  timestamp: number,
): SystemMessage {
  const d = (data ?? {}) as {
    summary?: unknown
    tokens_before?: unknown
    timestamp?: unknown
  }
  const tokensBefore = typeof d.tokens_before === 'number' ? d.tokens_before : 0
  return {
    id: entryId,
    role: 'system',
    kind: 'compaction',
    content:
      tokensBefore > 0
        ? `compacted ${tokensBefore.toLocaleString()} tokens`
        : 'conversation compacted',
    tone: 'info',
    summaryText: typeof d.summary === 'string' ? d.summary : undefined,
    tokensBefore,
    createdAt: typeof d.timestamp === 'number' ? d.timestamp : timestamp,
  }
}

/**
 * Derive the UI segments for one transcript item. `function_result` entries
 * return [] — they pair into an existing function-call row instead (see
 * applyEntryUpsert).
 */
export function entrySegments(
  item: TranscriptItem,
  sessionId?: string,
): Message[] {
  if (item.custom) {
    return (
      customSegments(
        item.entry_id,
        item.custom.custom_type,
        item.custom.data,
        Date.now(),
      ) ?? []
    )
  }
  const message = item.message
  if (!message) return []

  switch (message.role) {
    case 'user': {
      const origin = item.origin as
        | { notification?: unknown; reaction?: unknown; spawn?: unknown }
        | undefined
      const isNotif =
        origin?.notification === true || item.entry_id.startsWith('e_notify_')
      // A react-fired task delivered into this session (origin on events,
      // `e_react_` prefix on reads — session::messages carries no origin).
      const isReaction =
        origin?.reaction === true || item.entry_id.startsWith('e_react_')
      // A direct `harness::spawn` seed task — same pattern, `e_spawn_` prefix.
      const isSpawn =
        origin?.spawn === true || item.entry_id.startsWith('e_spawn_')
      const { text, attachments } = splitUserContent(message.content)
      const split = isReaction ? splitReactionTask(text) : { task: text }
      const msg: UserMessage = {
        id: item.entry_id,
        role: 'user',
        content: split.task,
        createdAt: message.timestamp,
        ...(attachments.length > 0 ? { attachments } : {}),
        ...(isNotif ? { notification: true } : {}),
        ...(isReaction ? { reaction: true } : {}),
        ...(isSpawn ? { spawn: true } : {}),
        ...(split.appendix ? { reactionEvent: split.appendix } : {}),
      }
      return [msg]
    }
    case 'assistant':
      return assistantSegments(item.entry_id, message, sessionId)
    case 'function_result':
      return []
    case 'custom': {
      // `session::messages` read-backs surface kind:custom entries as a
      // `role: 'custom'` message (`custom_type` + `details`) — the same
      // records some paths deliver as `item.custom`. Dispatch typed records
      // through the shared mapper first; anything unrecognized falls back to
      // its display text.
      const typed = customSegments(
        item.entry_id,
        message.custom_type,
        message.details,
        message.timestamp,
      )
      if (typed) return typed
      const content = message.display ?? textOf(message.content)
      if (!content) return []
      const msg: SystemMessage = {
        id: item.entry_id,
        role: 'system',
        kind: 'notice',
        content,
        tone: 'info',
        createdAt: message.timestamp,
      }
      return [msg]
    }
  }
}

function assistantSegments(
  entryId: string,
  message: Extract<AgentMessage, { role: 'assistant' }>,
  sessionId?: string,
): Message[] {
  const out: Message[] = []
  for (const [i, block] of message.content.entries()) {
    const id = `${entryId}:${i}`
    switch (block.type) {
      case 'thinking':
        out.push({
          id,
          role: 'thought',
          content: block.text,
          durationMs: 0,
          createdAt: message.timestamp,
        })
        break
      case 'text':
        if (block.text.length === 0) break
        out.push({
          id,
          role: 'assistant',
          content: block.text,
          model: message.model,
          createdAt: message.timestamp,
        })
        break
      case 'function_call': {
        const { functionId, input, unresolvedTarget } =
          unwrapFunctionCall(block)
        const msg: FunctionCallMessage = {
          id,
          role: 'function-call',
          functionId,
          input,
          unresolvedTarget,
          functionCallId: block.id,
          sessionId,
          createdAt: message.timestamp,
        }
        out.push(msg)
        break
      }
      case 'image':
      case 'function_result':
        break
    }
  }
  return out
}

/** Non-error output mirrors `function_execution_end.result`. */
export function functionResultOutput(
  message: Extract<AgentMessage, { role: 'function_result' }>,
): unknown {
  if (message.is_error) {
    return {
      error: {
        kind: 'function_error',
        message:
          textOf(message.content).replace(/\s+/g, ' ').trim() ||
          'function returned an error',
        details: message.details,
        content: message.content,
      },
    }
  }
  return { content: message.content, details: message.details }
}

function belongsToEntry(messageId: string, entryId: string): boolean {
  return messageId === entryId || messageId.startsWith(`${entryId}:`)
}

/** Patchable transient state for a function-call row. */
export type FcallPatch = Partial<
  Pick<
    FunctionCallMessage,
    | 'running'
    | 'pendingApproval'
    | 'output'
    | 'durationMs'
    | 'sessionId'
    | 'functionCallId'
    | 'filesystemAccess'
  >
>

/**
 * Patch the function-call row matching `functionCallId`. Returns the same
 * array when no row matched (caller may then append a fallback row).
 */
export function applyFcallPatch(
  messages: Message[],
  functionCallId: string,
  patch: FcallPatch,
): { messages: Message[]; found: boolean } {
  let found = false
  const next = messages.map((m) => {
    if (m.role !== 'function-call' || m.functionCallId !== functionCallId)
      return m
    found = true
    return { ...m, ...patch } as Message
  })
  return { messages: found ? next : messages, found }
}

/**
 * Upsert one transcript item into the ordered message list:
 * - replaces the entry's existing segment range in place (or appends at the
 *   end — appends only ever happen at the active leaf);
 * - absorbs transient state (and locally-created duplicate rows) by
 *   `functionCallId`;
 * - pairs `function_result` entries into their function-call row;
 * - infers `running` for unpaired function calls while the session is
 *   `working` (the real backend emits no execution start/end events, so
 *   the transcript shape is the only in-flight signal).
 */
export function applyEntryUpsert(
  messages: Message[],
  item: TranscriptItem,
  opts?: { sessionId?: string; streaming?: boolean; working?: boolean },
): Message[] {
  // function_result: fill the matching call row instead of inserting.
  if (item.message?.role === 'function_result') {
    const output = functionResultOutput(item.message)
    const { messages: patched, found } = applyFcallPatch(
      messages,
      item.message.function_call_id,
      { output, running: false, pendingApproval: false },
    )
    if (found) return patched
    // Fallback (assistant snapshot lost): standalone row carrying the result.
    const row: FunctionCallMessage = {
      id: item.entry_id,
      role: 'function-call',
      functionId: item.message.function_id,
      input: undefined,
      output,
      functionCallId: item.message.function_call_id,
      sessionId: opts?.sessionId,
      createdAt: item.message.timestamp,
    }
    return [...messages, row]
  }

  let segments = entrySegments(item, opts?.sessionId)

  // Carry over transient/local state for function-call segments and drop the
  // locally-created rows they replace (pending-approval fallback rows, which
  // have non-entry ids).
  const absorbedLocalIds = new Set<string>()
  segments = segments.map((segment) => {
    if (segment.role !== 'function-call' || !segment.functionCallId)
      return segment
    const existing = messages.find(
      (m): m is FunctionCallMessage =>
        m.role === 'function-call' &&
        m.functionCallId === segment.functionCallId,
    )
    if (!existing) return segment
    if (!belongsToEntry(existing.id, item.entry_id))
      absorbedLocalIds.add(existing.id)
    return {
      ...segment,
      output: existing.output,
      durationMs: existing.durationMs,
      running: existing.running,
      pendingApproval: existing.pendingApproval,
      sessionId: existing.sessionId ?? segment.sessionId,
      filesystemAccess: existing.filesystemAccess ?? segment.filesystemAccess,
    }
  })

  // While the session is working, an unpaired call (no result yet, not held
  // for approval) is in flight: surface the running state. Pairing a
  // `function_result` flips it off; `clearTransientFlags` sweeps leftovers
  // when the session leaves `working`.
  if (opts?.working) {
    segments = segments.map((segment) => {
      if (
        segment.role !== 'function-call' ||
        segment.running ||
        segment.pendingApproval ||
        segment.output !== undefined
      )
        return segment
      return { ...segment, running: true }
    })
  }

  // Preserve optimistic-only fields when replacing a user message in place.
  // Snapshot-derived attachments (collapsed `<attached-file>` blocks) win —
  // they are the durable truth; optimistic chips only fill the gap.
  segments = segments.map((segment) => {
    if (segment.role !== 'user') return segment
    if (segment.attachments) return segment
    const existing = messages.find(
      (m): m is UserMessage => m.role === 'user' && m.id === segment.id,
    )
    if (!existing?.attachments) return segment
    return { ...segment, attachments: existing.attachments }
  })

  if (opts?.streaming) {
    const last = segments[segments.length - 1]
    if (last && (last.role === 'assistant' || last.role === 'thought')) {
      segments = [
        ...segments.slice(0, -1),
        { ...last, streaming: true } as Message,
      ]
    }
  }

  const withoutAbsorbed = absorbedLocalIds.size
    ? messages.filter((m) => !absorbedLocalIds.has(m.id))
    : messages

  const firstIdx = withoutAbsorbed.findIndex((m) =>
    belongsToEntry(m.id, item.entry_id),
  )
  if (firstIdx === -1) {
    return segments.length > 0
      ? [...withoutAbsorbed, ...segments]
      : withoutAbsorbed
  }
  const before = withoutAbsorbed.slice(0, firstIdx)
  const after = withoutAbsorbed
    .slice(firstIdx)
    .filter((m) => !belongsToEntry(m.id, item.entry_id))
  return [...before, ...segments, ...after]
}

/** Full hydration: fold the active path into an ordered message list. */
export function transcriptToMessages(
  items: TranscriptItem[],
  sessionId?: string,
  opts?: { working?: boolean },
): Message[] {
  // Only the last assistant entry can carry in-flight calls; unpaired calls
  // in earlier entries are historical (interrupted turns) and must not
  // pulse. Result entries folded after it clear the calls that completed.
  let lastAssistantIdx = -1
  if (opts?.working) {
    for (let i = items.length - 1; i >= 0; i--) {
      if (items[i].message?.role === 'assistant') {
        lastAssistantIdx = i
        break
      }
    }
  }
  let messages: Message[] = []
  for (const [i, item] of items.entries()) {
    messages = applyEntryUpsert(messages, item, {
      sessionId,
      working: i === lastAssistantIdx,
    })
  }
  return messages
}

/** Clear transient streaming/running flags (turn over, abort, error). */
export function clearTransientFlags(messages: Message[]): Message[] {
  let changed = false
  const next = messages.map((m) => {
    if ((m.role === 'assistant' || m.role === 'thought') && m.streaming) {
      changed = true
      return { ...m, streaming: false }
    }
    if (m.role === 'function-call' && m.running) {
      changed = true
      return { ...m, running: false }
    }
    return m
  })
  return changed ? next : messages
}
