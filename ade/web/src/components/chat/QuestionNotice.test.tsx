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
      root.render(<QuestionNotice count={2} direction="up" onShow={onShow} />),
    )
    const button = container.querySelector('button')
    expect(button?.getAttribute('type')).toBe('button')
    expect(button?.textContent).toBe('2 questions waiting for your answerShow')
    await act(() => button?.click())
    expect(onShow).toHaveBeenCalledTimes(1)
  })
})
