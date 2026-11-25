// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { QuestionNotice, questionNoticeText } from './QuestionNotice'

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

describe('QuestionNotice', () => {
  it('counts the questions', () => {
    expect(questionNoticeText(1)).toBe('1 question waiting for your answer')
    expect(questionNoticeText(3)).toBe('3 questions waiting for your answer')
  })

  it('is one native button, named by its words, that jumps', async () => {
    const onShow = vi.fn()
    await act(() =>
      root.render(
        <QuestionNotice notice={{ count: 2, direction: 'up', onShow }} />,
      ),
    )
    const button = container.querySelector('button')
    expect(button?.getAttribute('type')).toBe('button')
    expect(button?.textContent).toBe('2 questions waiting for your answerShow')
    await act(() => button?.click())
    expect(onShow).toHaveBeenCalledTimes(1)
  })

  it('keeps a polite status mounted, so the notice is announced as it appears', async () => {
    await act(() => root.render(<QuestionNotice notice={null} />))
    const status = container.querySelector('[role="status"]')
    expect(status?.getAttribute('aria-live')).toBe('polite')
    expect(status?.textContent).toBe('')
    expect(container.querySelector('button')).toBeNull()

    await act(() =>
      root.render(
        <QuestionNotice
          notice={{ count: 1, direction: 'down', onShow: () => {} }}
        />,
      ),
    )
    // The same node, now carrying the words (and not the control).
    expect(container.querySelector('[role="status"]')).toBe(status)
    expect(status?.textContent).toBe('1 question waiting for your answer')
  })
})
