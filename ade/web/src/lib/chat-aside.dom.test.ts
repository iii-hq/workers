// @vitest-environment jsdom

import { afterEach, beforeAll, describe, expect, it } from 'vitest'
import type { Message } from '@/types/chat'
import { findTextRange, selectionQuote } from './chat-aside'

function prose(html: string): HTMLElement {
  const root = document.createElement('div')
  root.innerHTML = html
  return root
}

describe('findTextRange', () => {
  it('finds a passage that crosses inline elements', () => {
    const root = prose(
      '<p>The views stay <strong>in the browser</strong>, per pane.</p>',
    )
    expect(findTextRange(root, 'stay in the browser, per')?.toString()).toBe(
      'stay in the browser, per',
    )
  })

  it('ignores the newlines a selection adds between blocks', () => {
    const root = prose('<p>First paragraph.</p><ul><li>One item</li></ul>')
    expect(findTextRange(root, 'paragraph.\n\nOne item')?.toString()).toBe(
      'paragraph.One item',
    )
  })

  it('returns null when the text is not there', () => {
    expect(findTextRange(prose('<p>abc</p>'), '**abc**')).toBeNull()
    expect(findTextRange(prose('<p>abc</p>'), '   ')).toBeNull()
  })
})

describe('selectionQuote', () => {
  const messages = [
    { id: 'm1', role: 'assistant', content: '', agent: '', createdAt: 1 },
    { id: 'm2', role: 'user', content: '', createdAt: 2 },
  ] as Message[]
  let root: HTMLElement

  beforeAll(() => {
    // jsdom lays nothing out.
    Range.prototype.getBoundingClientRect ??= () => new DOMRect()
  })
  afterEach(() => {
    window.getSelection()?.removeAllRanges()
    root?.remove()
  })

  function select(fromEl: Element, from: number, toEl: Element, to: number) {
    const range = document.createRange()
    range.setStart(fromEl.firstChild as Node, from)
    range.setEnd(toEl.firstChild as Node, to)
    window.getSelection()?.removeAllRanges()
    window.getSelection()?.addRange(range)
  }

  function transcript() {
    root = document.createElement('section')
    root.innerHTML =
      '<div data-message-row="m1"><p>The views stay local.</p></div>' +
      '<div data-message-row="m2"><p>Do they sync?</p></div>'
    document.body.appendChild(root)
    return [...root.querySelectorAll('p')]
  }

  it('quotes a selection inside one message, named by its agent', () => {
    const [reply] = transcript()
    select(reply, 4, reply, 9)
    expect(selectionQuote(root, messages, 'Frontend')?.quote).toEqual({
      messageId: 'm1',
      text: 'views',
      agent: 'Frontend',
    })
  })

  it('names no agent on a user message', () => {
    const [, question] = transcript()
    select(question, 0, question, 2)
    expect(selectionQuote(root, messages, 'Frontend')?.quote.agent).toBe(
      undefined,
    )
  })

  it('ignores a selection across messages, outside the root, or empty', () => {
    const [reply, question] = transcript()
    select(reply, 0, question, 2)
    expect(selectionQuote(root, messages)).toBeNull()
    select(reply, 0, reply, 3)
    expect(selectionQuote(document.createElement('div'), messages)).toBeNull()
    window.getSelection()?.collapse(reply.firstChild as Node, 1)
    expect(selectionQuote(root, messages)).toBeNull()
  })
})
