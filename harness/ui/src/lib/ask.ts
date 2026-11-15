/**
 * Pure parsing for `harness::ask` calls: what the ask card draws. No React,
 * no host.
 *
 * The shapes come from ade/web/src/lib/sessions/entry-mapper.ts:
 * - `message.input` is the model's arguments: the agent_call `payload`
 *   for a wrapped call (plus `_streaming` while it streams), the raw
 *   arguments for a native call, and `undefined` on a paged-out
 *   (`unloaded`) row.
 * - `message.output` is `functionResultOutput`: `{ content, details }` for a
 *   result (the relayed envelope may also carry `terminate`), and
 *   `{ error: { kind, message, details, content } }` for an `is_error` one
 *   (a refused ask). It is absent while the call runs.
 *
 * The questions come from the result `details` (`ask::awaiting_result`):
 * the copy the harness validated and showed. `input` is not read. It is the
 * unvalidated model copy, and it can be missing or still streaming while the
 * result is already on record.
 */

import type { FunctionCallMessage } from '@iii-dev/console-ui'
import { unwrapEnvelope } from '@iii-dev/console-ui/format'

export interface AskOptionView {
  label: string
  description?: string
}

export interface AskQuestionView {
  header: string
  question: string
  multiSelect: boolean
  options: AskOptionView[]
}

export interface AskView {
  /** The function-call id of the ask. */
  questionId: string
  sessionId: string
  /** The turn that asked; once the session's current turn differs, the
   *  question has been answered. */
  turnId: string
  questions: AskQuestionView[]
}

/** The only `details.status` an accepted ask records. */
const AWAITING_ANSWER = 'awaiting_answer'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** A string that is not empty or whitespace, as the harness validates. */
function isText(value: unknown): value is string {
  return typeof value === 'string' && value.trim().length > 0
}

function parseOption(value: unknown): AskOptionView | null {
  if (!isRecord(value) || !isText(value.label)) return null
  const { label, description } = value
  // Omitted when absent; the schema also allows `null`.
  if (description == null || description === '') return { label }
  return typeof description === 'string' ? { label, description } : null
}

function parseQuestion(value: unknown): AskQuestionView | null {
  if (!isRecord(value) || !isText(value.header) || !isText(value.question)) {
    return null
  }
  // Always present in `details`; `false` is the schema default.
  const multiSelect = value.multi_select ?? false
  if (typeof multiSelect !== 'boolean') return null
  if (!Array.isArray(value.options) || value.options.length === 0) return null
  const options: AskOptionView[] = []
  for (const raw of value.options) {
    const option = parseOption(raw)
    if (!option) return null
    options.push(option)
  }
  return { header: value.header, question: value.question, multiSelect, options }
}

/**
 * The card for a settled, accepted `harness::ask` call, or `null` (fall
 * through) for a running call with no output, an error output, a missing or
 * unknown `details.status`, or malformed ids or questions.
 */
export function parseAsk(
  message: Pick<FunctionCallMessage, 'output'>,
): AskView | null {
  const { output } = message
  if (!isRecord(output) || 'error' in output) return null
  const details = unwrapEnvelope(output)
  if (!isRecord(details) || details.status !== AWAITING_ANSWER) return null
  const { question_id, session_id, turn_id, questions } = details
  if (!isText(question_id) || !isText(session_id) || !isText(turn_id)) {
    return null
  }
  if (!Array.isArray(questions) || questions.length === 0) return null
  const parsed: AskQuestionView[] = []
  for (const raw of questions) {
    const question = parseQuestion(raw)
    if (!question) return null
    parsed.push(question)
  }
  return {
    questionId: question_id,
    sessionId: session_id,
    turnId: turn_id,
    questions: parsed,
  }
}

/**
 * What the card holds for one question; `formatAnswer` takes one per
 * question, by position.
 *
 * - `picked`: the option labels the user ticked, in any (click) order. A
 *   single-select holds at most one; if it holds more, the first in
 *   option order counts. A label that is not one of the question's options
 *   is ignored.
 * - `other`: the free-text "Other" field as typed (a single-line input).
 *   On a single-select a non-blank `other` replaces the pick, so the card
 *   clears `other` when the user ticks an option, and clears `picked`
 *   when they choose Other.
 * - `skipped`: the user chose not to answer this question. It counts as
 *   answered for sending, with `SKIPPED_ANSWER` as the answer, whatever
 *   `picked` and `other` hold.
 */
export interface AskSelection {
  picked: readonly string[]
  other?: string
  skipped?: boolean
}

/** The answer line a skipped question sends (`Header: (skipped)`). The
 *  harness tells the model what it means (ask.rs `AWAITING_TEXT`). */
export const SKIPPED_ANSWER = '(skipped)'

/**
 * The message Dismiss sends instead of answers. The model learns from the
 * ask result that a dismissal may arrive, so this reads as that notice.
 */
export const DISMISS_TEXT =
  'I dismissed your questions without answering them. Go on with your best judgement; ask again later only if you really must.'

/** One question's answer, or null when it has none. */
function answerFor(
  question: AskQuestionView,
  selection: AskSelection | undefined,
): string | null {
  if (!selection) return null
  if (selection.skipped) return SKIPPED_ANSWER
  const other = selection.other?.trim() ?? ''
  // Option order, not click order.
  const picked = question.options
    .map((option) => option.label)
    .filter((label) => selection.picked.includes(label))
  if (!question.multiSelect) return other || picked[0] || null
  const items = other ? [...picked, other] : picked
  return items.length > 0 ? items.join(', ') : null
}

/**
 * The message the card sends: one `Header: answer` line per question, in
 * question order. The answer is the picked label (single-select), the
 * picked labels joined with ", " in option order plus the Other text last
 * (multi-select), the trimmed Other text, or `SKIPPED_ANSWER` for a skipped
 * question. `null` while any question is unanswered (not skipped, no pick
 * and a blank Other); never an empty string.
 */
export function formatAnswer(
  view: AskView,
  selections: readonly AskSelection[],
): string | null {
  if (view.questions.length === 0) return null
  const lines: string[] = []
  for (const [index, question] of view.questions.entries()) {
    const answer = answerFor(question, selections[index])
    if (answer === null) return null
    lines.push(`${question.header}: ${answer}`)
  }
  return lines.join('\n')
}

/** What became of a card: the answer was sent, or it was dismissed. */
export type AskOutcome = 'answered' | 'dismissed'

/**
 * The settled cards of a session, from `harness::ask::settlements`
 * (`{ settlements: [{ question_id, outcome, settled_at }] }`), keyed by
 * question id. `undefined` when the reply cannot be read (an older harness
 * without the function, a failed call): nothing is known, so every card
 * stays open, which is better than closing a question that still needs an
 * answer.
 */
export function settlementsOf(reply: unknown): ReadonlyMap<string, AskOutcome> | undefined {
  if (!isRecord(reply) || !Array.isArray(reply.settlements)) return undefined
  const settled = new Map<string, AskOutcome>()
  for (const row of reply.settlements) {
    if (!isRecord(row) || !isText(row.question_id)) continue
    if (row.outcome === 'answered' || row.outcome === 'dismissed') {
      settled.set(row.question_id, row.outcome)
    }
  }
  return settled
}

/**
 * Where the session stands, as far as the card can read it from a verbose
 * `harness::status` reply: the current turn, its step, and the text of every
 * message parked in its queue. `null` is a session with no turn record (it
 * expired, or there never was one); `undefined` is unknown (not read yet, a
 * failed call, or a shape the card cannot read).
 */
export interface TurnPosition {
  turnId: string
  step: number
  queuedTexts: readonly string[]
}
export type CurrentTurn = TurnPosition | null | undefined

export function turnPositionOf(report: unknown): CurrentTurn {
  if (report === null) return null
  if (!isRecord(report) || !isText(report.turn_id)) return undefined
  const step = typeof report.step === 'number' && Number.isFinite(report.step) ? report.step : 0
  const queuedTexts: string[] = []
  if (Array.isArray(report.queued)) {
    for (const row of report.queued) {
      const text = queuedTextOf(row)
      if (text !== null) queuedTexts.push(text)
    }
  }
  return { turnId: report.turn_id, step, queuedTexts }
}

/** The text blocks of a queued row's message, joined with newlines. */
function queuedTextOf(row: unknown): string | null {
  if (!isRecord(row) || !isRecord(row.message) || !Array.isArray(row.message.content)) {
    return null
  }
  const parts: string[] = []
  for (const block of row.message.content) {
    if (isRecord(block) && block.type === 'text' && typeof block.text === 'string') {
      parts.push(block.text)
    }
  }
  return parts.join('\n')
}

/**
 * Whether the chat moved past the turn that asked. Only a hint on an open
 * card ("you can still answer"): a later turn is no proof the question was
 * answered, which is exactly why settlement, not the turn, closes a card.
 */
export function movedOn(view: AskView, turn: CurrentTurn): boolean {
  return turn === null || (turn !== undefined && turn.turnId !== view.turnId)
}

/**
 * Whether a Send reached the session: `compose` returns nothing, and the
 * composer drops a submit while it is blocked (no model chosen), leaving
 * the text as a draft. Read against where the session stood when the card
 * sent (`before`) and a status read after it (`after`), the message landed
 * when
 * - it is parked in the queue (sent while a step streamed), or
 * - the session is on another turn (it seeded a new one), or
 * - the same turn stepped on (it drained into a running turn).
 * An unknown read after proves nothing; without a position before, only
 * the queue can tell.
 */
export function sendLanded(before: CurrentTurn, after: CurrentTurn, text: string): boolean {
  if (after === undefined) return false
  if (after === null) return false
  if (after.queuedTexts.includes(text)) return true
  if (before === undefined) return false
  if (before === null) return true
  return after.turnId !== before.turnId || after.step > before.step
}

/**
 * Where the card's own Send (or Dismiss) stands:
 * - `idle`: nothing sent from this card;
 * - `sending`: the text went to the composer and the card is re-reading
 *   the session;
 * - `unconfirmed`: the last of those reads still showed nothing landed;
 * - `failed`: `compose` threw.
 */
export type SendPhase = 'idle' | 'sending' | 'unconfirmed' | 'failed'

/**
 * What the card shows:
 * - `open`: takes an answer;
 * - `sending`: waiting for the send to land, with the buttons disabled so
 *   nothing goes out twice;
 * - `answered`: read-only, the answer was sent;
 * - `dismissed`: read-only, dismissed without an answer;
 * - `not-sent`: takes an answer again; the composer most likely kept the
 *   text as a draft;
 * - `failed`: takes an answer again; `compose` threw.
 */
export type AskCardState = 'open' | 'sending' | 'answered' | 'dismissed' | 'not-sent' | 'failed'

/**
 * The card's state. Only a settlement closes it: this card's own confirmed
 * Send or Dismiss, or one recorded earlier (another tab, before a reload).
 * The session moving to a later turn does not: the person may have said
 * something else, and the question still needs its answer.
 */
export function askCardState(
  settlement: AskOutcome | undefined,
  phase: SendPhase,
): AskCardState {
  if (settlement !== undefined) return settlement
  switch (phase) {
    case 'sending':
      return 'sending'
    case 'unconfirmed':
      return 'not-sent'
    case 'failed':
      return 'failed'
    case 'idle':
      return 'open'
  }
}
