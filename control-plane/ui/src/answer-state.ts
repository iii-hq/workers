import type { AskEntry, AskQuestion, StructuredAnswer } from '../../src/types'

export interface Selection {
  picked: string[]
  text: string
}

export function recommendedSelection(question: AskQuestion): Selection {
  return {
    picked: question.options.filter((option) => option.recommended).map((option) => option.label),
    text: '',
  }
}

export function selectOption(question: AskQuestion, selection: Selection, label: string): Selection {
  return {
    picked: question.multi_select
      ? selection.picked.includes(label)
        ? selection.picked.filter((picked) => picked !== label)
        : [...selection.picked, label]
      : [label],
    text: '',
  }
}

export function typeAnswer(question: AskQuestion, text: string): Selection {
  return text ? { picked: [], text } : recommendedSelection(question)
}

export function selectedAnswers(questions: AskQuestion[], selections: Selection[]): StructuredAnswer[] | null {
  const answers: StructuredAnswer[] = []
  for (const [index, question] of questions.entries()) {
    const selection = selections[index]
    if (!selection) return null
    const text = selection.text.trim()
    const picked = question.options
      .filter((option) => selection.picked.includes(option.label))
      .map((option) => option.label)
    if (!text && !picked.length) return null
    answers.push({
      header: question.header,
      ...(text ? { other: text } : question.multi_select ? { choices: picked } : { choice: picked[0] }),
    })
  }
  return answers.length ? answers : null
}

export type AnswerState = {
  status: 'idle' | 'sending' | 'failed' | 'delivered' | 'already' | 'withdrawn' | 'not-delivered'
  summary?: string
  error?: string
  channel?: string
}

type AnswerEvent =
  | { type: 'send'; summary: string }
  | { type: 'failed'; error: string }
  | { type: 'settled'; entry: AskEntry; own?: boolean }
  | { type: 'delivered' }

export function isRetryableAnswerError(error: string) {
  return /ask_busy:\s*expiry notice in delivery,\s*retry/i.test(error)
}

export function answerTransition(state: AnswerState, event: AnswerEvent): AnswerState {
  if (event.type === 'settled') {
    const entry = event.entry
    if (!['idle', 'sending', 'failed'].includes(state.status) && ['open', 'expired', 'answering'].includes(entry.state))
      return state
    const summary = entry.answer_text ?? state.summary
    if (entry.state === 'withdrawn') return { status: 'withdrawn', summary }
    if (entry.outcome === 'not_delivered') return { status: 'not-delivered', summary }
    if (entry.state === 'answered' || entry.state === 'dismissed') {
      return {
        status: event.own || state.status === 'delivered' ? 'delivered' : 'already',
        summary,
        channel: entry.by?.channel,
      }
    }
    if (entry.state === 'answering' && entry.by?.channel === 'expiry' && !entry.outcome)
      return state.status === 'sending' ? state : { status: 'idle' }
    if (entry.state === 'answering') return { status: 'sending', summary }
    return state
  }
  if (event.type === 'delivered') return { ...state, status: 'delivered' }
  if (event.type === 'send') {
    return state.status === 'idle' || state.status === 'failed' ? { status: 'sending', summary: event.summary } : state
  }
  if (state.status !== 'sending') return state
  if (isRetryableAnswerError(event.error)) return state
  if (/withdrawn/i.test(event.error)) return { ...state, status: 'withdrawn' }
  const answered = /already answered(?: by (\w+))?/i.exec(event.error)
  return answered
    ? { ...state, status: 'already', channel: answered[1] }
    : { ...state, status: 'failed', error: event.error }
}
