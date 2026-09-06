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
 *   function_call → function-trigger row). Each `message-updated` snapshot
 *   re-derives the segment list wholesale and replaces the entry's range.
 * - `function_result` entries render no row of their own; they fill the
 *   `output` of the function-trigger row with the matching `functionTriggerId`.
 * - Lifecycle custom entries (`error`, `recovery`, `reaction`) render durable
 *   system notices so failure state survives refresh.
 * - `custom_type: "compaction"` custom entries render the compaction marker.
 *
 * Events are at-least-once and unordered: callers keep the highest
 * `revision` per entry (see use-conversations) and treat `session::messages`
 * read-backs as the source of truth.
 */

import { parseAttachedFileHeader } from '@/lib/file-mentions'
import { parseSlashBlockHeader, slashChip } from '@/lib/slash-commands'
import type {
  Attachment,
  FunctionTriggerMessage,
  Message,
  SystemMessage,
  SystemNoticeFailure,
  SystemNoticeTechnicalDetails,
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
export const RECOVERY_CUSTOM_TYPE = 'recovery'
export const REACTION_CUSTOM_TYPE = 'reaction'

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
      return [
        lifecycleNotice(entryId, customType, data, timestamp) ??
          customNotice(entryId, data, 'error', 'turn failed', timestamp),
      ]
    case NOTICE_CUSTOM_TYPE:
      return [customNotice(entryId, data, 'info', 'notice', timestamp)]
    case RECOVERY_CUSTOM_TYPE:
    case REACTION_CUSTOM_TYPE: {
      const notice = lifecycleNotice(entryId, customType, data, timestamp)
      return notice ? [notice] : []
    }
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

/** The trigger's display name: label, else state scope/key. */
export function triggerFiredName(t: TriggerFiredData): string {
  if (t.label) return t.label
  if (t.key) return t.scope ? `${t.scope}/${t.key}` : t.key
  return t.trigger_type ?? 'trigger'
}

/** The compact timeline copy: an explicit event action, then the binding's
 * existing display name for historical registrations. */
export function triggerFiredEventText(t: TriggerFiredData): string {
  const action = t.action?.trim()
  return action || triggerFiredName(t)
}

/**
 * Plain one-liner for the fired notice (also the a11y/fallback content).
 * Generic over the delivery target: a wake reads "notified this chat", any
 * function target reads "called <fn>". The legacy branch keeps historical
 * `'spawn'` records (written before spawn stopped being a binding target)
 * rendering as they did.
 */
export function triggerFiredSummary(t: TriggerFiredData): string {
  const name = triggerFiredEventText(t)
  const deliveredAction =
    t.target === 'spawn'
      ? `spawned${t.model ? ` ${t.model}` : ''}`
      : !t.target || t.target === 'notify' || t.target === 'harness::send'
        ? 'notified this chat'
        : `called ${t.target}`
  const lifecycle = triggerLifecycleSummary(t)
  // Lifecycle-only records did not attempt their target. Keeping a call or
  // notification verb in their fallback summary would invent a delivery.
  if (
    t.outcome === 'expired' ||
    t.outcome === 'unregistered' ||
    t.outcome === 'invalidated'
  ) {
    return `${name} · ${lifecycle ?? 'binding retired'}`
  }
  const action =
    t.outcome === 'skipped'
      ? 'delivery skipped'
      : t.outcome === 'delivery_failed'
        ? !t.target || t.target === 'notify' || t.target === 'harness::send'
          ? 'notification failed'
          : `call to ${t.target} failed`
        : deliveredAction
  return `${name} · ${action}${lifecycle ? ` · ${lifecycle}` : ''}`
}

function triggerLifecycleSummary(t: TriggerFiredData): string | null {
  switch (t.retirement_reason) {
    case 'once_consumed':
      return 'once consumed'
    case 'max_fires':
      return 'delivery limit reached'
    case 'expired':
      return 'binding expired'
    case 'unregistered':
      return 'binding manually removed'
    case 'invalidated':
      return 'binding invalidated'
    case 'exhausted':
      return 'binding exhausted'
  }
  if (!t.retired) return null
  // Historical successful one-shot records predate the structured reason;
  // "consumed" is accurate, while "unregistered" falsely implies a person.
  if (t.once && (!t.outcome || t.outcome === 'delivered'))
    return 'once consumed'
  return 'binding retired'
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

function lifecycleNotice(
  entryId: string,
  customType: string,
  data: unknown,
  timestamp: number,
): SystemMessage | null {
  const d = (data ?? {}) as Record<string, unknown>
  const status = typeof d.status === 'string' ? d.status : undefined
  const summary =
    typeof d.summary === 'string'
      ? d.summary
      : typeof d.reason === 'string'
        ? d.reason
        : undefined
  const createdAt = typeof d.timestamp === 'number' ? d.timestamp : timestamp

  if (customType === ERROR_CUSTOM_TYPE) {
    const code = typeof d.code === 'string' ? d.code : undefined
    const errorClass = typeof d.class === 'string' ? d.class : undefined
    const detail =
      typeof d.detail === 'string'
        ? d.detail
        : typeof d.reason === 'string'
          ? d.reason
          : typeof d.message === 'string'
            ? d.message
            : undefined
    const provider = typeof d.provider === 'string' ? d.provider : undefined
    const model = typeof d.model === 'string' ? d.model : undefined
    const technicalDetails: SystemNoticeTechnicalDetails = {
      code,
      class: errorClass,
      detail,
      provider,
      model,
    }
    const hasTechnicalDetails = Object.values(technicalDetails).some(Boolean)
    const nextActions = Array.isArray(d.next_actions)
      ? d.next_actions.filter(
          (action): action is string =>
            typeof action === 'string' && action.trim().length > 0,
        )
      : []
    const publicSummary =
      typeof d.summary === 'string'
        ? d.summary
        : 'The response could not be completed.'
    const partial = d.partial_result_available === true
    const recovery =
      d.recovery && typeof d.recovery === 'object'
        ? (d.recovery as Record<string, unknown>)
        : undefined
    const attempted =
      typeof recovery?.attempted === 'number' ? recovery.attempted : 0
    const maxAttempts =
      typeof recovery?.max_attempts === 'number'
        ? recovery.max_attempts
        : undefined
    const parts = [
      publicSummary,
      partial
        ? 'A partial response was preserved in this conversation and may be incomplete.'
        : undefined,
      attempted > 0
        ? `Automatic recovery stopped after ${attempted} of ${maxAttempts ?? attempted} attempts.`
        : undefined,
    ].filter((part): part is string => Boolean(part))
    const failure: SystemNoticeFailure = {
      summary: publicSummary,
      ...(typeof d.retryable === 'boolean' ? { retryable: d.retryable } : {}),
      ...(partial ? { partialResultAvailable: true } : {}),
      ...(attempted > 0
        ? {
            recoveryAttempted: attempted,
            ...(maxAttempts !== undefined
              ? { recoveryMaxAttempts: maxAttempts }
              : {}),
          }
        : {}),
      ...(typeof d.phase === 'string' ? { phase: d.phase } : {}),
    }
    return {
      id: entryId,
      role: 'system',
      kind: 'turn-failure',
      content: parts.join(' '),
      tone: 'error',
      failure,
      ...(nextActions.length > 0 ? { nextActions } : {}),
      ...(hasTechnicalDetails ? { technicalDetails } : {}),
      createdAt,
    }
  }

  if (customType === RECOVERY_CUSTOM_TYPE) {
    return {
      id: entryId,
      role: 'system',
      kind: 'notice',
      content:
        summary ??
        (status === 'recovered'
          ? 'generation recovered after a transient interruption'
          : 'generation interrupted; attempting recovery'),
      tone: status === 'recovered' ? 'info' : 'warn',
      createdAt,
    }
  }

  if (customType === REACTION_CUSTOM_TYPE) {
    const label =
      status === 'spawned'
        ? 'reaction spawned'
        : status === 'called'
          ? 'reaction called'
          : status === 'waiting'
            ? 'reaction waiting'
            : status === 'blocked'
              ? 'reaction blocked'
              : 'reaction failed'
    return {
      id: entryId,
      role: 'system',
      kind: 'notice',
      content: summary ? `${label} — ${summary}` : label,
      tone:
        status === 'blocked' || status === 'failed'
          ? 'error'
          : status === 'waiting'
            ? 'warn'
            : 'info',
      createdAt,
    }
  }

  return null
}

/** The harness wraps every tool call in agent_trigger; unwrap for display. */
function unwrapFunctionTrigger(
  block: Extract<ContentBlock, { type: 'function_call' }>,
): {
  functionId: string
  input: unknown
  description?: string
  unresolvedTarget?: boolean
} {
  if (block.function_id === 'agent_trigger') {
    if (block.arguments && typeof block.arguments === 'object') {
      const args = block.arguments as {
        function?: unknown
        description?: unknown
        payload?: unknown
        _streaming?: unknown
      }
      // Mid-stream, the harness supplies both incrementally reconstructed
      // fields and a bounded raw tail. Preserve the structured payload for
      // the request pane while `_streaming` keeps its live-state treatment.
      const streaming =
        typeof args._streaming === 'string' ? args._streaming : undefined
      const description =
        typeof args.description === 'string' &&
        args.description.trim().length > 0
          ? args.description.trim()
          : undefined
      if (typeof args.function === 'string' && args.function.length > 0) {
        if (streaming !== undefined) {
          const input =
            args.payload &&
            typeof args.payload === 'object' &&
            !Array.isArray(args.payload)
              ? {
                  ...(args.payload as Record<string, unknown>),
                  _streaming: streaming,
                }
              : { _streaming: streaming }
          return {
            functionId: args.function,
            input,
            description,
          }
        }
        return {
          functionId: args.function,
          input: args.payload ?? {},
          description,
        }
      }
      if (streaming !== undefined) {
        return {
          functionId: block.function_id,
          input: { _streaming: streaming },
          description,
          unresolvedTarget: true,
        }
      }
    }
    // Before the incremental parser has observed a non-empty target (or for
    // malformed provider output), render a placeholder instead of the literal
    // `agent_trigger`; the next snapshot self-corrects as soon as a value is
    // available.
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
 * `<attached-file …>` blocks are console-authored `#file(...)` mention and
 * document expansions — rendering their full content in the user bubble would
 * dump whole files into the chat, so they collapse to chips instead (failure
 * placeholders keep the error visible in the chip name). `<skill>` blocks
 * are slash expansions and collapse the same way — the typed command stays
 * the visible text, the body becomes a chip.
 *
 * An image block is the picture itself, sent to a vision model. It becomes a
 * chip carrying its own thumbnail: without this a conversation reloaded from
 * history shows the question and no sign that a screenshot went with it.
 */
function splitUserContent(blocks: ContentBlock[]): {
  text: string
  attachments: Attachment[]
} {
  let text = ''
  const attachments: Attachment[] = []
  let imageIndex = 0
  for (const block of blocks) {
    if (block.type === 'image') {
      imageIndex += 1
      const mime = block.mime || 'image/png'
      attachments.push({
        id: `image-${imageIndex}`,
        name: `image ${imageIndex}`,
        // Base64 inflates by a third; the original byte count is what a
        // person recognises, so report that rather than the encoded length.
        size: Math.floor((block.data?.length ?? 0) * 0.75),
        type: mime,
        dataUrl: block.data ? `data:${mime};base64,${block.data}` : undefined,
      })
      continue
    }
    if (block.type !== 'text') continue
    const header = parseAttachedFileHeader(block.text)
    if (header) {
      attachments.push({
        id: `mention-${header.path}`,
        name: header.error ? `${header.path} (${header.error})` : header.path,
        size: header.size ?? 0,
        type: 'text/x-file-mention',
      })
      continue
    }
    const slash = parseSlashBlockHeader(block.text)
    if (slash) {
      attachments.push(slashChip(slash, block.text.length))
    } else {
      text += block.text
    }
  }
  return { text, attachments }
}

/**
 * `harness::spawn` appends the firing event (or a join's gathered inputs) to
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
 * return [] — they pair into an existing function-trigger row instead (see
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
      // Internal recovery prompt sent back to the model. The paired durable
      // `recovery` custom entry explains the attempt to the user without
      // making this machine-authored instruction look human-authored.
      if (item.entry_id.includes('_transient_resume_')) return []
      const origin = item.origin as
        | {
            notification?: unknown
            binding?: unknown
            reaction?: unknown
            spawn?: unknown
            validation?: unknown
            skill_update?: unknown
          }
        | undefined
      const { text, attachments } = splitUserContent(message.content)
      if (
        origin?.skill_update === true ||
        /^e_.+_skills_\d+$/.test(item.entry_id)
      ) {
        return [
          {
            id: item.entry_id,
            role: 'system',
            kind: 'notice',
            tone: 'info',
            content: text,
            createdAt: message.timestamp,
          },
        ]
      }
      const isNotif =
        origin?.notification === true ||
        /^(?:e_notify_|e_fire_|e_expire_|e_stalespawn_|e_claimfail_|e_condfail_)/.test(
          item.entry_id,
        )
      const triggerBindingId = isNotif
        ? notificationBindingId(item.entry_id, origin?.binding)
        : undefined
      // A react-fired task delivered into this session (origin on events;
      // persisted reads carry no origin, so recognize both entry formats).
      const isReaction =
        origin?.reaction === true ||
        item.entry_id.startsWith('e_spawned_') ||
        item.entry_id.startsWith('e_react_')
      // A direct `harness::spawn` seed task — same pattern, `e_spawn_` prefix.
      const isSpawn =
        origin?.spawn === true || item.entry_id.startsWith('e_spawn_')
      // A validation nudge — the harness re-prompting after the output
      // contract or a post-turn validator rejected the result. Persisted
      // entries are `e_<turn_id>_nudge_<attempt>`.
      const isValidation =
        origin?.validation === true || /_nudge_\d+$/.test(item.entry_id)
      const split = isReaction ? splitReactionTask(text) : { task: text }
      const msg: UserMessage = {
        id: item.entry_id,
        role: 'user',
        content: split.task,
        createdAt: message.timestamp,
        ...(attachments.length > 0 ? { attachments } : {}),
        ...(isNotif ? { notification: true } : {}),
        ...(triggerBindingId ? { triggerBindingId } : {}),
        ...(isReaction ? { reaction: true } : {}),
        ...(isSpawn ? { spawn: true } : {}),
        ...(isValidation ? { validation: true } : {}),
        ...(split.appendix ? { reactionEvent: split.appendix } : {}),
      }
      return [msg]
    }
    case 'assistant': {
      const segments = assistantSegments(item.entry_id, message, sessionId)
      // Hook annotations from the entry origin: which memory bank and
      // memories fed this generate. Tag the first assistant segment so
      // the chat renders one memory chip per reply.
      const md = item.origin as
        | {
            memory_bank?: unknown
            memory_recalled?: unknown
            memory_ids?: unknown
            memory_rules?: unknown
            memory_rules_truncated?: unknown
            memory_retrieval?: unknown
          }
        | undefined
      if (typeof md?.memory_bank === 'string') {
        const first = segments.find((s) => s.role === 'assistant')
        if (first && first.role === 'assistant') {
          first.memory = {
            bank: md.memory_bank,
            memories:
              typeof md.memory_recalled === 'number' ? md.memory_recalled : 0,
            memoryIds: Array.isArray(md.memory_ids)
              ? md.memory_ids.filter((v): v is string => typeof v === 'string')
              : [],
            ...(typeof md.memory_rules === 'number'
              ? { rules: md.memory_rules }
              : {}),
            ...(md.memory_rules_truncated === true ? { truncated: true } : {}),
            ...(md.memory_retrieval === 'bm25-entity-semantic'
              ? { semantic: true }
              : {}),
          }
        }
      }
      return segments
    }
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

/**
 * Recover the binding identity from the trusted origin first, then from the
 * deterministic ids used by current and historical Harness versions.
 */
export function notificationBindingId(
  entryId: string,
  originBinding?: unknown,
): string | undefined {
  if (typeof originBinding === 'string' && originBinding) return originBinding

  const ordinalFire = /^e_fire_(.+)_\d+$/.exec(entryId)
  if (ordinalFire?.[1]) return ordinalFire[1]

  for (const prefix of [
    'e_expire_',
    'e_stalespawn_',
    'e_claimfail_',
    'e_condfail_',
    'e_notify_',
  ]) {
    if (entryId.startsWith(prefix)) {
      const id = entryId.slice(prefix.length)
      return id || undefined
    }
  }
  return undefined
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
          agent: message.agent,
          model: message.model,
          stopReason: message.stop_reason,
          createdAt: message.timestamp,
        })
        break
      case 'function_call': {
        const { functionId, input, description, unresolvedTarget } =
          unwrapFunctionTrigger(block)
        const msg: FunctionTriggerMessage = {
          id,
          role: 'function-trigger',
          functionId,
          description,
          input,
          unresolvedTarget,
          functionTriggerId: block.id,
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

/** Patchable transient state for a function-trigger row. */
export type FcallPatch = Partial<
  Pick<
    FunctionTriggerMessage,
    | 'running'
    | 'pendingApproval'
    | 'output'
    | 'durationMs'
    | 'sessionId'
    | 'functionTriggerId'
    | 'filesystemAccess'
  >
>

/**
 * Patch the function-trigger row matching `functionTriggerId`. Returns the same
 * array when no row matched (caller may then append a fallback row).
 */
export function applyFcallPatch(
  messages: Message[],
  functionTriggerId: string,
  patch: FcallPatch,
): { messages: Message[]; found: boolean } {
  let found = false
  const next = messages.map((m) => {
    if (
      m.role !== 'function-trigger' ||
      m.functionTriggerId !== functionTriggerId
    )
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
 *   `functionTriggerId`;
 * - pairs `function_result` entries into their function-trigger row;
 * - infers `running` for unpaired function triggers while the session is
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
    const row: FunctionTriggerMessage = {
      id: item.entry_id,
      role: 'function-trigger',
      functionId: item.message.function_id,
      input: undefined,
      output,
      functionTriggerId: item.message.function_call_id,
      sessionId: opts?.sessionId,
      createdAt: item.message.timestamp,
    }
    return [...messages, row]
  }

  let segments = entrySegments(item, opts?.sessionId)

  // Carry over transient/local state for function-trigger segments and drop the
  // locally-created rows they replace (pending-approval fallback rows, which
  // have non-entry ids).
  const absorbedLocalIds = new Set<string>()
  segments = segments.map((segment) => {
    if (segment.role !== 'function-trigger' || !segment.functionTriggerId)
      return segment
    const existing = messages.find(
      (m): m is FunctionTriggerMessage =>
        m.role === 'function-trigger' &&
        m.functionTriggerId === segment.functionTriggerId,
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
        segment.role !== 'function-trigger' ||
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
    if (m.role === 'function-trigger' && m.running) {
      changed = true
      return { ...m, running: false }
    }
    return m
  })
  return changed ? next : messages
}
