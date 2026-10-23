// @vitest-environment jsdom

import {
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from 'vitest'
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

  it('names the message of a row that also holds a notification', () => {
    root = document.createElement('section')
    root.innerHTML =
      '<div data-message-row="note-1 m1"><p>The views stay local.</p></div>'
    document.body.appendChild(root)
    const reply = root.querySelector('p') as HTMLElement
    select(reply, 0, reply, 3)
    expect(selectionQuote(root, messages, 'Frontend')?.quote).toEqual({
      messageId: 'm1',
      text: 'The',
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

describe('sideChatsLeftByClosedTabs', () => {
  const lock = 'iii-ade-instance'
  let held: Array<{ name: string }>
  const request = vi.fn()

  beforeEach(() => {
    vi.resetModules()
    localStorage.clear()
    sessionStorage.clear()
    held = []
    request.mockReset()
    vi.stubGlobal('navigator', {
      locks: { query: async () => ({ held }), request },
    })
  })
  afterEach(() => vi.unstubAllGlobals())

  async function fresh() {
    const aside = await import('./chat-aside')
    aside.sideChatsMadeHere.add('s1')
    aside.sideChatsMadeHere.add('s2')
    return aside
  }

  it('hands the first tab after all closed what they left, once', async () => {
    const aside = await fresh()
    expect(await aside.sideChatsLeftByClosedTabs()).toEqual(['s1', 's2'])
    expect(request).toHaveBeenCalledWith(
      lock,
      { mode: 'shared' },
      expect.any(Function),
    )
    expect(await aside.sideChatsLeftByClosedTabs()).toEqual(['s1', 's2'])
  })

  it('hands nothing to a reload or while another tab is open', async () => {
    sessionStorage.setItem('iii-ade-tab', '1')
    let aside = await fresh()
    expect(await aside.sideChatsLeftByClosedTabs()).toEqual([])
    vi.resetModules()
    sessionStorage.clear()
    held = [{ name: lock }]
    aside = await import('./chat-aside')
    expect(await aside.sideChatsLeftByClosedTabs()).toEqual([])
    expect(aside.sideChatsMadeHere.has('s1')).toBe(true)
  })

  it('hands nothing without the Web Locks API', async () => {
    vi.stubGlobal('navigator', {})
    const aside = await fresh()
    expect(await aside.sideChatsLeftByClosedTabs()).toEqual([])
  })
})

describe('shownInAnyTab', () => {
  afterEach(() => vi.unstubAllGlobals())

  it('sees what this tab and the other tabs show, and forgets a release', async () => {
    vi.resetModules()
    const requested: string[] = []
    let held = [{ name: 'iii-ade-shown:other-tab-chat' }]
    vi.stubGlobal('navigator', {
      locks: {
        query: async () => ({ held }),
        request: (name: string) => {
          requested.push(name)
          return new Promise(() => {})
        },
      },
    })
    const aside = await import('./chat-aside')
    const release = aside.markDisplayed('mine')
    aside.markDisplayed('mine')
    expect(requested).toEqual(['iii-ade-shown:mine'])
    expect([...(await aside.shownInAnyTab())].sort()).toEqual([
      'mine',
      'other-tab-chat',
    ])
    release()
    held = []
    expect([...(await aside.shownInAnyTab())]).toEqual(['mine'])
  })
})
