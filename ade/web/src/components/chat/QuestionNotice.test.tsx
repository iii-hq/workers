// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { QuestionNotice, questionNoticeText } from './QuestionNotice'
import type { AwaitingQuestion } from './use-awaiting-questions'

let container: HTMLDivElement
let root: Root

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(async () => {
  await act(() => root.unmount())
  container.remove()
  vi.unstubAllGlobals()
})

const question = (n: number, extra: Partial<AwaitingQuestion> = {}) => ({
  messageId: `fc_${n}`,
  title: `Question ${n}?`,
  questionCount: 1,
  ...extra,
})

describe('QuestionNotice', () => {
  it('counts the questions in a few words', () => {
    expect(questionNoticeText(1)).toBe('1 unanswered question')
    expect(questionNoticeText(3)).toBe('3 unanswered questions')
  })

  it('one question: one row that shows it', async () => {
    const jumps = { next: vi.fn(), jumpTo: vi.fn() }
    await act(() =>
      root.render(
        <QuestionNotice
          questions={[question(1)]}
          notice={{ next: 'fc_1', direction: 'up', jumps }}
        />,
      ),
    )
    const buttons = container.querySelectorAll('button')
    expect(buttons).toHaveLength(1)
    expect(buttons[0].textContent).toBe('1 unanswered questionShow')
    await act(() => buttons[0].click())
    expect(jumps.next).toHaveBeenCalledTimes(1)
  })

  it('several: the row goes to the next, the chevron lists them all to jump to', async () => {
    const jumps = { next: vi.fn(), jumpTo: vi.fn() }
    await act(() =>
      root.render(
        <QuestionNotice
          questions={[
            question(1, { header: 'Colour' }),
            question(2, { questionCount: 3 }),
          ]}
          notice={{ next: 'fc_1', direction: 'down', jumps }}
        />,
      ),
    )
    const row = container.querySelector<HTMLButtonElement>(
      '[data-question-jump]',
    )
    expect(row?.textContent).toBe('2 unanswered questionsNext')
    const toggle = container.querySelector<HTMLButtonElement>(
      'button[aria-expanded]',
    )
    expect(toggle?.getAttribute('aria-label')).toBe('List the questions')
    expect(toggle?.getAttribute('aria-expanded')).toBe('false')

    await act(() => toggle?.click())
    expect(toggle?.getAttribute('aria-expanded')).toBe('true')
    const items = container.querySelectorAll<HTMLButtonElement>(
      '[data-question-jump-to]',
    )
    expect([...items].map((item) => item.textContent)).toEqual([
      '1Colour · Question 1?',
      '2Question 2?+2 more',
    ])
    await act(() => items[1].click())
    expect(jumps.jumpTo).toHaveBeenCalledWith('fc_2')

    // Escape closes the list.
    await act(() => {
      items[1].dispatchEvent(
        new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }),
      )
    })
    expect(container.querySelector('[data-question-jump-to]')).toBeNull()
  })

  it('keeps a polite status mounted, so the notice is announced as it appears', async () => {
    await act(() =>
      root.render(<QuestionNotice questions={[question(1)]} notice={null} />),
    )
    const status = container.querySelector('[role="status"]')
    expect(status?.getAttribute('aria-live')).toBe('polite')
    expect(status?.textContent).toBe('')
    expect(container.querySelector('button')).toBeNull()

    await act(() =>
      root.render(
        <QuestionNotice
          questions={[question(1)]}
          notice={{
            next: 'fc_1',
            direction: 'down',
            jumps: { next: () => {}, jumpTo: () => {} },
          }}
        />,
      ),
    )
    // The same node, now carrying the words (and not the controls).
    expect(container.querySelector('[role="status"]')).toBe(status)
    expect(status?.textContent).toBe('1 unanswered question')
  })
})
