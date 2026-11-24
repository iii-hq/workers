export type AskQuestion = {
  header: string
  question: string
  multi_select: boolean
  options: { label: string; description?: string; recommended?: boolean }[]
}

export type Question = {
  session_id: string
  question_id: string
  questions: AskQuestion[]
  about: { ticket?: string; blocks: string[]; class: string; why?: string } | null
  asked_at: number
}

export type QuestionAnswer = {
  answer_text: string
  by: { channel: string; user?: string } | null
  outcome: 'answered' | 'dismissed' | 'withdrawn' | 'expired_default' | 'not_delivered'
}

/** The closed harness entry returned by a successful kanban::question::answer. */
export type QuestionAnswerResult = Question & QuestionAnswer & {
  state: 'answered' | 'dismissed'
  settled_at: number
}

export type StructuredAnswer = { header: string } &
  ({ choice: string } | { choices: string[] } | { other: string })

export type Selection = { picked: string[]; other: string }

export function textSelection(question: AskQuestion, other = ''): Selection {
  return {
    picked: other ? [] : question.options.filter((option) => option.recommended === true).map((option) => option.label),
    other,
  }
}

export function pickOption(question: AskQuestion, selection: Selection, label: string): Selection {
  return {
    picked: question.multi_select
      ? selection.picked.includes(label) ? selection.picked.filter((picked) => picked !== label) : [...selection.picked, label]
      : [label],
    other: '',
  }
}

export function questionAnswers(questions: AskQuestion[], selections: Selection[]): StructuredAnswer[] | null {
  if (questions.length === 0) return null
  const answers: StructuredAnswer[] = []
  for (const [index, question] of questions.entries()) {
    const selection = selections[index]
    if (!selection) return null
    const header = question.header
    const other = selection.other.trim()
    const choices = question.options.map((option) => option.label).filter((label) => selection.picked.includes(label))
    if (other) answers.push({ header, other })
    else if (choices.length) answers.push(question.multi_select ? { header, choices } : { header, choice: choices[0]! })
    else return null
  }
  return answers
}

export function mayDismiss(question: Question): boolean {
  return ['scope', 'routine', 'run_health'].includes(question.about?.class ?? 'scope')
}
