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
 * - An `elided` item (a paged read left the inside of a long tool-call run
 *   out) produces the same rows with `unloaded: true` and no arguments or
 *   output; the whole entry from `session::messages-range` later replaces
 *   them in place through the same identity scheme, so the swap is invisible.
 * - Lifecycle custom entries (`error`, `recovery`, `reaction`) render durable
 *   system notices so failure state survives refresh.
 * - `custom_type: "compaction"` custom entries render the compaction marker.
 *
 * Events are at-least-once and unordered: callers keep the highest
 * `revision` per entry (see use-conversations) and treat `session::messages`
 * read-backs as the source of truth.
 */

import { attachedFileLabel, parseAttachedFileHeader } from '@/lib/file-mentions'
import { parseSkillUpdate } from '@/lib/skill-update'
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
 * When the read left the bytes out (`data: ""`, `attachment_id` set) the
 * chip carries the id instead and the thumbnail is fetched when the chip
 * scrolls into view; a chip with neither `dataUrl` nor `file` but an
 * `attachmentId` is what tells the renderer the bytes live in the store.
 *
 * A `file` block is a reference to the original bytes session-manager kept.
 * The console sends it ALONGSIDE the expansion of the same file, so a message
 * would otherwise grow two chips per attachment; `mergeFileChips` folds each
 * pair into the one chip that can hand the original back out.
 */
function splitUserContent(blocks: ContentBlock[]): {
  text: string
  attachments: Attachment[]
} {
  let text = ''
  const slots: ChipSlot[] = []
  let imageIndex = 0
  for (const block of blocks) {
    if (block.type === 'image') {
      imageIndex += 1
      const mime = block.mime || 'image/png'
      // The bytes, when present, are still what the chip is drawn from — an
      // old worker that ignored `include_image_data` lands here and renders
      // as it always did. The id rides along either way: it pairs the block
      // with its `file` reference below, and when the bytes were left out it
      // is all the chip has to fetch them by.
      const attachmentId = block.attachment_id || undefined
      slots.push({
        kind: 'image',
        chip: {
          id: attachmentId ?? `image-${imageIndex}`,
          name: `image ${imageIndex}`,
          // Base64 inflates by a third; the original byte count is what a
          // person recognises, so report that rather than the encoded length.
          size: Math.floor((block.data?.length ?? 0) * 0.75),
          type: mime,
          dataUrl: block.data ? `data:${mime};base64,${block.data}` : undefined,
          ...(attachmentId ? { attachmentId } : {}),
        },
      })
      continue
    }
    if (block.type === 'file') {
      slots.push({
        kind: 'file',
        mime: block.mime,
        chip: {
          id: block.attachment_id,
          name: block.name,
          size: block.size,
          type: block.mime,
          attachmentId: block.attachment_id,
        },
      })
      continue
    }
    if (block.type !== 'text') continue
    const header = parseAttachedFileHeader(block.text)
    if (header) {
      const label = attachedFileLabel(header)
      slots.push({
        kind: 'mention',
        path: header.path,
        chip: {
          id: `mention-${label}`,
          name: header.error ? `${label} (${header.error})` : label,
          size: header.size ?? 0,
          type: 'text/x-file-mention',
        },
      })
      continue
    }
    const slash = parseSlashBlockHeader(block.text)
    if (slash) {
      slots.push({ kind: 'slash', chip: slashChip(slash, block.text.length) })
    } else {
      text += block.text
    }
  }
  return { text, attachments: mergeFileChips(slots) }
}

/** One chip and where it came from, so a `file` reference can find its twin. */
type ChipSlot =
  | { kind: 'image'; chip: Attachment }
  | { kind: 'mention'; path: string; chip: Attachment }
  | { kind: 'file'; mime: string; chip: Attachment }
  | { kind: 'slash'; chip: Attachment }

/**
 * One chip per attachment, however many blocks it became on the wire.
 *
 * A document goes out as an `<attached-file path="…">` expansion plus a
 * `file` reference to the same name; a picture as an image block plus a
 * `file` reference with an image type. The reference is the chip that
 * survives — it is the one that carries the real name, the real size and the
 * id the bytes live under — but it takes what only its twin knows: the
 * mention's label when that says more than the name (a line range, the
 * reason a read failed), the image's thumbnail.
 *
 * Image blocks carry no name. One that names its stored original pairs with
 * the reference of the same id; the rest pair by order alone. That holds
 * because the send path writes both lists in attachment order, and a picture
 * that never became an image block (refused for a model without vision, or
 * too large) went out as a named failure expansion instead, which the name
 * match above claims first. An image whose bytes were left out of the read
 * brings nothing but its id to the merge: the reference already carries it,
 * and the missing thumbnail is what marks the chip as one to fetch later.
 */
function mergeFileChips(slots: ChipSlot[]): Attachment[] {
  const files = slots.filter((s) => s.kind === 'file')
  if (files.length === 0) return slots.map((s) => s.chip)

  const absorbed = new Set<ChipSlot>()
  const unpairedImages = slots.filter((s) => s.kind === 'image')
  for (const file of files) {
    const mention = slots.find(
      (s): s is Extract<ChipSlot, { kind: 'mention' }> =>
        s.kind === 'mention' && !absorbed.has(s) && s.path === file.chip.name,
    )
    if (mention) {
      absorbed.add(mention)
      if (mention.chip.name !== file.chip.name) {
        file.chip = { ...file.chip, name: mention.chip.name }
      }
      continue
    }
    if (!file.mime.startsWith('image/')) continue
    const byId = unpairedImages.findIndex(
      (s) => s.chip.attachmentId === file.chip.attachmentId,
    )
    const image =
      byId >= 0 ? unpairedImages.splice(byId, 1)[0] : unpairedImages.shift()
    if (!image) continue
    absorbed.add(image)
    if (image.chip.dataUrl) {
      file.chip = { ...file.chip, dataUrl: image.chip.dataUrl }
    }
  }
  return slots.filter((s) => !absorbed.has(s)).map((s) => s.chip)
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
        // The harness re-sent the model its skill index. The model needs the
        // whole block; the reader gets a one-line marker with the list behind
        // a disclosure. An unrecognised shape stays a plain notice.
        const skills = parseSkillUpdate(text)
        return [
          skills
            ? {
                id: item.entry_id,
                role: 'system',
                kind: 'skills',
                tone: skills.available ? 'info' : 'warn',
                content: text,
                skills,
                createdAt: message.timestamp,
              }
            : {
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
      const segments = assistantSegments(
        item.entry_id,
        message,
        sessionId,
        item.elided === true,
      )
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
  elided = false,
): Message[] {
  const out: Message[] = []
  for (const [i, block] of message.content.entries()) {
    const id = `${entryId}:${i}`
    // A placeholder call: the page kept the block's id and function id and
    // emptied the arguments. Not unwrapped — with `arguments: {}` an
    // `agent_trigger` wrapper has no target to unwrap to, so the row keeps
    // the wrapper name until its (elided) result names the real function.
    if (block.type === 'function_call' && elided) {
      const msg: FunctionTriggerMessage = {
        id,
        role: 'function-trigger',
        functionId: block.function_id,
        input: undefined,
        unloaded: true,
        ...(block.function_id === 'agent_trigger'
          ? { unresolvedTarget: true }
          : {}),
        functionTriggerId: block.id,
        sessionId,
        createdAt: message.timestamp,
      }
      out.push(msg)
      continue
    }
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

/** Whether a UI message id is one of `entryId`'s segments (see the header). */
export function belongsToEntry(messageId: string, entryId: string): boolean {
  return messageId === entryId || messageId.startsWith(`${entryId}:`)
}

/** The transcript entry a UI message id was derived from. */
export function entryIdOfMessage(messageId: string): string {
  const colon = messageId.indexOf(':')
  return colon === -1 ? messageId : messageId.slice(0, colon)
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
    | 'functionId'
    | 'unresolvedTarget'
    | 'unloaded'
    | 'resultEntryId'
  >
>

/**
 * Patch the function-trigger row matching `functionTriggerId`. Returns the same
 * array when no row matched (caller may then append a fallback row).
 */
export function applyFcallPatch(
  messages: Message[],
  functionTriggerId: string,
  patch: FcallPatch | ((row: FunctionTriggerMessage) => FcallPatch),
): { messages: Message[]; found: boolean } {
  let found = false
  const next = messages.map((m) => {
    if (
      m.role !== 'function-trigger' ||
      m.functionTriggerId !== functionTriggerId
    )
      return m
    found = true
    return {
      ...m,
      ...(typeof patch === 'function' ? patch(m) : patch),
    } as Message
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
    // An elided result settles the row without an output: the call is over
    // (not running, not held), and the body is a placeholder until a range
    // read brings the whole entry. The result's `function_id` is the real
    // target — the harness resolves `agent_trigger` before recording it —
    // so a wrapper-named placeholder learns its label here. A whole result
    // clears the flag, since it is what the flag was waiting for.
    const elided = item.elided === true
    const result = item.message
    const settled: FcallPatch = {
      running: false,
      pendingApproval: false,
      resultEntryId: item.entry_id,
    }
    const patch = elided
      ? (row: FunctionTriggerMessage): FcallPatch =>
          // A re-read page elides a result this window already holds: the
          // output stays, and so does the settled label.
          row.output !== undefined
            ? settled
            : {
                ...settled,
                unloaded: true,
                functionId: result.function_id,
                unresolvedTarget: false,
              }
      : { ...settled, output: functionResultOutput(result), unloaded: false }
    const { messages: patched, found } = applyFcallPatch(
      messages,
      result.function_call_id,
      patch,
    )
    if (found) return patched
    // Fallback (assistant snapshot lost): standalone row carrying the result.
    const row: FunctionTriggerMessage = {
      id: item.entry_id,
      role: 'function-trigger',
      functionId: result.function_id,
      input: undefined,
      ...(elided
        ? { unloaded: true }
        : { output: functionResultOutput(result) }),
      resultEntryId: item.entry_id,
      functionTriggerId: result.function_call_id,
      sessionId: opts?.sessionId,
      createdAt: result.timestamp,
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
    // A re-read page elides a call this window already holds whole (a
    // reconnect re-hydrates the tail). Nothing on the page is newer than the
    // row: keep it, under the page's segment id.
    if (segment.unloaded && !existing.unloaded) {
      return { ...existing, id: segment.id, createdAt: segment.createdAt }
    }
    // A whole entry replacing a placeholder brings the arguments, but the
    // result is its own entry. If that result is known and still unfetched,
    // the row stays a placeholder until it lands; a placeholder with no
    // result on record has none coming (an interrupted call) and settles.
    const stillUnloaded =
      segment.unloaded === true ||
      (existing.unloaded === true &&
        existing.resultEntryId !== undefined &&
        existing.output === undefined)
    return {
      ...segment,
      output: existing.output,
      durationMs: existing.durationMs,
      running: existing.running,
      pendingApproval: existing.pendingApproval,
      sessionId: existing.sessionId ?? segment.sessionId,
      filesystemAccess: existing.filesystemAccess ?? segment.filesystemAccess,
      resultEntryId: existing.resultEntryId ?? segment.resultEntryId,
      ...(stillUnloaded ? { unloaded: true } : {}),
      // A re-read placeholder keeps the label its result already resolved;
      // the page itself still only knows the wrapper name.
      ...(segment.unloaded && existing.functionId !== 'agent_trigger'
        ? { functionId: existing.functionId, unresolvedTarget: false }
        : {}),
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
        segment.output !== undefined ||
        // No output because the page left it out, not because the call is
        // still going: a placeholder must never pulse.
        segment.unloaded
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

/**
 * Put an older page in front of the loaded window. The page is folded on its
 * own (its runs are whole, so results pair inside it) and then dropped in
 * above; a message the window already holds is left out of the older part,
 * since the anchor entry can sit on both sides of a page boundary and a
 * duplicate row would render twice. The live tail is untouched.
 */
export function prependTranscript(
  messages: Message[],
  items: TranscriptItem[],
  sessionId?: string,
): Message[] {
  if (items.length === 0) return messages
  const held = new Set(messages.map((m) => m.id))
  const older = transcriptToMessages(items, sessionId).filter(
    (m) => !held.has(m.id),
  )
  return older.length > 0 ? [...older, ...messages] : messages
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
