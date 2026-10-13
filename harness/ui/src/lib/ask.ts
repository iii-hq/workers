/**
 * Pure parsing for `harness::ask` calls: what the ask card draws. No React,
 * no host.
 *
 * The shapes come from ade/web/src/lib/sessions/entry-mapper.ts:
 * - `message.input` is the model's arguments: the agent_trigger `payload`
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

import type { FunctionTriggerMessage } from '@iii-dev/console-ui'
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
  message: Pick<FunctionTriggerMessage, 'output'>,
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
 *   single-select (radio) holds at most one; if it holds more, the first in
 *   option order counts. A label that is not one of the question's options
 *   is ignored.
 * - `other`: the free-text "Other" field as typed (a single-line input).
 *   On a single-select a non-blank `other` replaces the pick, so the card
 *   clears `other` when the user picks an option radio, and clears `picked`
 *   when they choose Other.
 */
export interface AskSelection {
  picked: readonly string[]
  other?: string
}

/** One question's answer, or null when it has none. */
function answerFor(
  question: AskQuestionView,
  selection: AskSelection | undefined,
): string | null {
  if (!selection) return null
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
 * (multi-select), or the trimmed Other text. `null` while any question is
 * unanswered (no pick and a blank Other); never an empty string.
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

/**
 * The session's current turn, as far as the card knows it:
 * - a turn id: the session's current turn;
 * - `null`: the session has no turn record (it expired, or there never was
 *   one);
 * - `undefined`: unknown (the status read has not answered yet, failed, or
 *   returned a shape the card cannot read).
 */
export type CurrentTurn = string | null | undefined

/**
 * The current turn in a lean `harness::status` reply. The handler answers
 * `null` when the session has no turn record and fills `turn_id` whenever it
 * has one (harness/src/functions/status.rs), so `null` means "no record" and
 * a reply without a usable `turn_id` proves nothing either way: unknown.
 */
export function turnIdOf(report: unknown): CurrentTurn {
  if (report === null) return null
  if (!isRecord(report)) return undefined
  return isText(report.turn_id) ? report.turn_id : undefined
}

/**
 * Whether the question still takes an answer: the session's current turn is
 * the one that asked. A session with no turn record counts as answered: a card
 * exists only because a turn ran, so a missing record means that turn is over
 * (its record expired). An unknown current turn (not read yet, or the read
 * failed) counts as open. It is better to allow an answer than to block one.
 */
export function isOpen(view: AskView, currentTurn: CurrentTurn): boolean {
  return currentTurn === undefined || currentTurn === view.turnId
}

/**
 * The current turn after a new status read. A turn that has moved past the
 * asking one (another turn id, or no record) never comes back, so once the
 * card has seen that, a later read (a failed one included) cannot reopen it.
 */
export function mergeTurnRead(
  askingTurnId: string,
  previous: CurrentTurn,
  read: CurrentTurn,
): CurrentTurn {
  return previous !== undefined && previous !== askingTurnId ? previous : read
}

/**
 * Where the card's own Send stands:
 * - `idle`: nothing sent from this card;
 * - `sending`: the answer went to the composer and the card is re-reading
 *   the turn;
 * - `unconfirmed`: the last of those reads came back with the asking turn
 *   still current;
 * - `failed`: `compose` threw.
 */
export type SendPhase = 'idle' | 'sending' | 'unconfirmed' | 'failed'

/**
 * What the card shows:
 * - `open`: takes an answer;
 * - `sending`: waiting for the next turn, with Send disabled so nothing goes
 *   out twice;
 * - `answered`: read-only;
 * - `not-sent`: takes an answer again; the composer most likely kept the
 *   text as a draft;
 * - `failed`: takes an answer again; `compose` threw.
 */
export type AskCardState = 'open' | 'sending' | 'answered' | 'not-sent' | 'failed'

/**
 * The card's state. Only the session's turn decides `answered`: it moved
 * past the turn that asked, or its record is gone (`isOpen`). A Send alone
 * never does: `compose` returns nothing, and the composer drops a submit
 * while it is blocked (no model chosen, a turn still streaming), leaving the
 * text as a draft.
 */
export function askCardState(
  view: AskView,
  currentTurn: CurrentTurn,
  phase: SendPhase,
): AskCardState {
  if (!isOpen(view, currentTurn)) return 'answered'
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
