import { describe, expect, it } from 'vitest'
import {
  answerTransition,
  recommendedSelection,
  selectedAnswers,
  selectOption,
  typeAnswer,
} from './answer-state'
import { towerFixture } from './fixtures'
import type { AskEntry, AskQuestion } from './model'

const question: AskQuestion = {
  header: 'Builds',
  question: 'Where?',
  options: [{ label: 'Sandbox', recommended: true }, { label: 'Host' }],
}
const entry = (
  state: AskEntry['state'],
  overrides: Partial<AskEntry> = {},
): AskEntry => ({
  session_id: 's',
  question_id: 'q',
  turn_id: 't',
  questions: [question],
  about: {},
  asked_at: 1,
  on_expiry: 'block',
  state,
  ...overrides,
})

describe('Tower answer selection', () => {
  it('only preselects a marked recommendation', () => {
    expect(recommendedSelection(question)).toEqual({
      picked: ['Sandbox'],
      text: '',
    })
    expect(
      recommendedSelection({
        ...question,
        options: [{ label: 'Host' }, { label: 'Sandbox' }],
      }),
    ).toEqual({ picked: [], text: '' })
  })
  it('typing clears every pick and clearing restores only the recommendation', () => {
    const picked = selectOption(
      question,
      recommendedSelection(question),
      'Host',
    )
    expect(picked.picked).toEqual(['Host'])
    expect(typeAnswer(question, 'somewhere else')).toEqual({
      picked: [],
      text: 'somewhere else',
    })
    expect(typeAnswer(question, '')).toEqual(recommendedSelection(question))
    expect(
      typeAnswer({ ...question, options: [{ label: 'Host' }] }, ''),
    ).toEqual({ picked: [], text: '' })
    expect(
      selectedAnswers([question], [typeAnswer(question, '   ')]),
    ).toBeNull()
  })
  it('sends multi-select in option order and requires an answer for each question', () => {
    const multi = { ...question, multi_select: true }
    const selected = selectOption(
      multi,
      selectOption(multi, { picked: [], text: '' }, 'Host'),
      'Sandbox',
    )
    expect(selectedAnswers([multi], [selected])).toEqual([
      { header: 'Builds', choices: ['Sandbox', 'Host'] },
    ])
    expect(selectOption(multi, selected, 'Host').picked).toEqual(['Sandbox'])
    expect(selectedAnswers([question, question], [selected])).toBeNull()
    expect(
      selectedAnswers([question], [typeAnswer(question, '  isolated VM  ')]),
    ).toEqual([{ header: 'Builds', other: 'isolated VM' }])
  })
})
describe('Tower answer state machine', () => {
  it('keeps the sent summary through failure and retry, then delivers', () => {
    const sending = answerTransition(
      { status: 'idle' },
      { type: 'send', summary: 'Builds: Sandbox' },
    )
    expect(sending.status).toBe('sending')
    const failed = answerTransition(sending, {
      type: 'failed',
      error: 'timeout',
    })
    expect(failed).toEqual({
      status: 'failed',
      summary: 'Builds: Sandbox',
      error: 'timeout',
    })
    const retry = answerTransition(failed, {
      type: 'send',
      summary: failed.summary ?? '',
    })
    expect(
      answerTransition(retry, {
        type: 'settled',
        entry: entry('answered', { answer_text: 'Builds: Sandbox' }),
        own: true,
      }).status,
    ).toBe('delivered')
  })
  it('closes from another channel and cannot be reopened by a late failed request', () => {
    const state = answerTransition(
      { status: 'sending' },
      {
        type: 'settled',
        entry: entry('answered', { by: { channel: 'kanban', user: 'user' } }),
      },
    )
    expect(state).toMatchObject({ status: 'already', channel: 'kanban' })
    expect(
      answerTransition(state, { type: 'failed', error: 'timeout' }),
    ).toEqual(state)
    expect(answerTransition(state, { type: 'send', summary: 'again' })).toEqual(
      state,
    )
    expect(
      answerTransition(state, { type: 'settled', entry: entry('open') }),
    ).toEqual(state)
  })
  it('distinguishes answering, expired, withdrawn and not delivered', () => {
    expect(
      answerTransition(
        { status: 'idle' },
        {
          type: 'settled',
          entry: entry('answering', { answer_text: 'reserved' }),
        },
      ),
    ).toEqual({ status: 'sending', summary: 'reserved' })
    expect(
      answerTransition(
        { status: 'idle' },
        { type: 'settled', entry: entry('expired') },
      ).status,
    ).toBe('idle')
    expect(
      answerTransition(
        { status: 'sending' },
        { type: 'settled', entry: entry('withdrawn') },
      ).status,
    ).toBe('withdrawn')
    expect(
      answerTransition(
        { status: 'sending' },
        {
          type: 'settled',
          entry: entry('answered', { outcome: 'not_delivered' }),
          own: true,
        },
      ).status,
    ).toBe('not-delivered')
    expect(
      answerTransition(
        { status: 'sending' },
        { type: 'failed', error: 'invalid request: already answered by chat' },
      ),
    ).toMatchObject({ status: 'already', channel: 'chat' })
    expect(
      answerTransition(
        { status: 'sending' },
        { type: 'failed', error: 'question withdrawn by the asker' },
      ).status,
    ).toBe('withdrawn')
  })
  it('provides no selection or recommendation on held calls', () => {
    const held = towerFixture.items.filter((item) => item.kind === 'held')
    expect(held.length).toBeGreaterThan(0)
    for (const item of held) {
      expect(item).not.toHaveProperty('entry')
      expect(item).not.toHaveProperty('recommended')
      expect(item).not.toHaveProperty('selection')
    }
  })
})
