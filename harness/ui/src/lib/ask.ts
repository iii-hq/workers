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
