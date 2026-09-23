// @vitest-environment jsdom

import { act, type ReactNode } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { MessageList } from '@/components/chat/MessageList'
import { ChatFileNavigation, openChatFile } from './file-navigation'
import { Markdown } from './markdown'
import { resetPanelContextForTests, subscribePanelOpen } from './panel-context'
import { registerExtPage } from './ui-slots'
import type { Message } from '@/types/chat'

let container: HTMLDivElement
let root: Root
const opened = vi.fn()
let removePage: () => void

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  resetPanelContextForTests()
  opened.mockReset()
  removePage = registerExtPage({ id: 'ide', title: 'IDE', scope: 'ide', path: 'ide/page.js', render: () => null })
  subscribePanelOpen(opened)
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(async () => {
  await act(() => root.unmount())
  container.remove()
  removePage()
  resetPanelContextForTests()
  vi.unstubAllGlobals()
})

async function render(children: ReactNode) {
  await act(() => root.render(children))
}

async function click(selector = 'button') {
  const button = container.querySelector<HTMLButtonElement>(selector)
  expect(button).not.toBeNull()
  await act(async () => { button?.click() })
}

function expectFile(path: string, line?: number, endLine?: number) {
  expect(opened).toHaveBeenLastCalledWith(
    expect.objectContaining({
      pageId: 'ide',
      context: { type: 'file', path, ...(line ? { line, endLine } : {}) },
    }),
  )
}

describe('chat file navigation', () => {
  it('opens the original config_override citation through the real transcript', async () => {
    await render(
      <ChatFileNavigation workingDir="/repo/config-override" enabled>
        <MessageList
          messages={[
            {
              id: 'citation',
              role: 'assistant',
              createdAt: 0,
              content:
                'Referência: [`crates/iii-compose/src/lifecycle.rs:1437–1463`](crates/iii-compose/src/lifecycle.rs#L1437).',
            },
          ]}
        />
      </ChatFileNavigation>,
    )
    const selector =
      'button[title^="open crates/iii-compose/src/lifecycle.rs:"]'
    const button = container.querySelector<HTMLButtonElement>(selector)
    expect(button?.type).toBe('button')
    expect(button?.tabIndex).toBe(0)
    expect(button?.querySelector('code')?.textContent).toContain('1437–1463')
    await click(selector)
    expectFile(
      '/repo/config-override/crates/iii-compose/src/lifecycle.rs',
      1437,
      1437,
    )
    expect(opened).toHaveBeenCalledTimes(1)
    expect(container.querySelector('a[href*="lifecycle.rs"]')).toBeNull()
  })

  it('opens line ranges and decoded absolute paths', async () => {
    await render(
      <ChatFileNavigation workingDir="/unrelated" enabled>
        <Markdown>{'[file](/repo/a%20b.ts#L12-L40)'}</Markdown>
      </ChatFileNavigation>,
    )
    await click()
    expectFile('/repo/a b.ts', 12, 40)
  })

  it('preserves filename:line destinations without allowing unsafe protocols', async () => {
    await render(
      <ChatFileNavigation workingDir="/repo" enabled>
        <Markdown>
          {'[file](README.md:12-40) [unsafe](javascript:alert)'}
        </Markdown>
      </ChatFileNavigation>,
    )
    await click()
    expectFile('/repo/README.md', 12, 40)
    expect(container.querySelector('a')?.getAttribute('href')).toBe('')
  })

  it('keeps the directory of each chat and follows directory changes', async () => {
    const chats = (dir: string) => (
      <>
        <ChatFileNavigation workingDir={dir} enabled>
          <Markdown>{'[first](src/a.ts)'}</Markdown>
        </ChatFileNavigation>
        <ChatFileNavigation workingDir="/second" enabled>
          <Markdown>{'[second](src/a.ts)'}</Markdown>
        </ChatFileNavigation>
      </>
    )
    await render(chats('/first'))
    await click()
    expectFile('/first/src/a.ts')
    await act(() =>
      container.querySelectorAll<HTMLButtonElement>('button')[1].click(),
    )
    expectFile('/second/src/a.ts')
    await render(chats('/changed'))
    await click()
    expectFile('/changed/src/a.ts')
  })

  it('opens #file mentions, but not folders or literal code', async () => {
    await render(
      <ChatFileNavigation workingDir="/repo/" enabled>
        <Markdown>
          {
            '#file(src/a.ts:12-40) #file(src/) `#file(src/b.ts)`\n\n```\n#file(src/c.ts)\n```'
          }
        </Markdown>
      </ChatFileNavigation>,
    )
    expect(container.querySelectorAll('button')).toHaveLength(1)
    await click()
    expectFile('/repo/src/a.ts', 12, 40)
  })

  it('preserves external links, fragments and routes without nesting interactive mentions', async () => {
    await render(
      <ChatFileNavigation workingDir="/repo" enabled>
        <Markdown>
          {
            '[web](https://example.com/a.ts#L2) [mail](mailto:user@example.com) [route](/workers) [section](#section) [#file(src/a.ts)](https://example.com)'
          }
        </Markdown>
      </ChatFileNavigation>,
    )
    expect(container.querySelectorAll('a')).toHaveLength(5)
    expect(container.querySelectorAll('button')).toHaveLength(0)
    const web = container.querySelector('a')
    expect(web?.getAttribute('href')).toBe('https://example.com/a.ts#L2')
    expect(web?.getAttribute('target')).toBe('_blank')
    expect(web?.getAttribute('rel')).toBe('noopener noreferrer')
    expect(opened).not.toHaveBeenCalled()
  })

  it.each([false, true])(
    'does not invent a workspace when unavailable (enabled=%s)',
    async (enabled) => {
      await render(
        <ChatFileNavigation
          workingDir={enabled ? null : '/repo'}
          enabled={enabled}
        >
          <Markdown>{'[file](src/a.ts) #file(src/a.ts)'}</Markdown>
        </ChatFileNavigation>,
      )
      expect(container.querySelector('a')).toBeNull()
      await click()
      expect(container.querySelector('[role="alert"]')?.textContent).toContain(enabled ? 'original folder' : 'disabled')
      expect(opened).not.toHaveBeenCalled()
    },
  )

  it('leaves shared Markdown previews outside a chat unchanged', async () => {
    await render(<Markdown>{'[file](src/a.ts) #file(src/a.ts)'}</Markdown>)
    expect(container.querySelector('button')).toBeNull()
    expect(container.querySelector('a')).not.toBeNull()
  })

  it('shares the opener with the composer and ignores folders or unresolved relative files', () => {
    openChatFile({ path: 'src/a.ts', range: { from: 2, to: 4 } }, '/repo')
    expectFile('/repo/src/a.ts', 2, 4)
    openChatFile({ path: '/repo/b.ts' }, null)
    expectFile('/repo/b.ts')
    expect(() => openChatFile({ path: 'src/' }, '/repo')).toThrow()
    expect(() => openChatFile({ path: 'src/a.ts' }, null)).toThrow()
    expect(opened).toHaveBeenCalledTimes(2)
  })
  it('opens absolute links without a chat folder', async () => {
    await render(<ChatFileNavigation workingDir={null} enabled><Markdown>{'[file](/repo/a.ts)'}</Markdown></ChatFileNavigation>)
    await click()
    expectFile('/repo/a.ts')
  })

  it('reports an absent IDE and succeeds after it reconnects', async () => {
    removePage()
    await render(<ChatFileNavigation workingDir="/repo" enabled><Markdown>{'[file](a.ts)'}</Markdown></ChatFileNavigation>)
    await click()
    expect(container.querySelector('[role="alert"]')?.textContent).toContain('IDE is not available')
    expect(opened).not.toHaveBeenCalled()
    removePage = registerExtPage({ id: 'ide', title: 'IDE', scope: 'ide', path: 'ide/page.js', render: () => null })
    await click()
    expectFile('/repo/a.ts')
    expect(container.querySelector('[role="alert"]')).toBeNull()
  })

  it('routes invalid references to an actionable error, never web navigation', async () => {
    await render(<ChatFileNavigation workingDir="/repo" enabled><Markdown>{'[bad](src/a.ts#L0)'}</Markdown></ChatFileNavigation>)
    expect(container.querySelector('a')).toBeNull()
    await click()
    expect(container.querySelector('[role="alert"]')?.textContent).toContain('Invalid file reference')
    expect(opened).not.toHaveBeenCalled()
  })

  it('uses historical scope rather than the current chat folder', async () => {
    const messages: Message[] = [
      { id: 'old', role: 'assistant', createdAt: 0, content: '[old](a.ts)' },
      { id: 'scope', role: 'system', kind: 'working-dir', createdAt: 1, content: '', scope: { path: '/new', previousPath: '/old', cause: 'selected' } },
      { id: 'new', role: 'assistant', createdAt: 2, content: '[new](a.ts)' },
    ]
    await render(<ChatFileNavigation workingDir="/unrelated" enabled messages={messages}><MessageList messages={messages} /></ChatFileNavigation>)
    await click('button[title^="open a.ts"]')
    expectFile('/old/a.ts')
    await act(() => container.querySelectorAll<HTMLButtonElement>('button[title^="open a.ts"]')[1].click())
    expectFile('/new/a.ts')
  })

  it('opens an unchanged conversation directly once its history is complete', async () => {
    const messages: Message[] = [{ id: 'initial', role: 'assistant', createdAt: 0, content: '[file](a.ts)' }]
    await render(<ChatFileNavigation workingDir="/repo" enabled messages={messages} historyComplete><MessageList messages={messages} /></ChatFileNavigation>)
    await click('button[title^="open a.ts"]')
    expectFile('/repo/a.ts')
    expect(document.querySelector('[role="alertdialog"]')).toBeNull()
  })

  it.each([false, true])('keeps confirmation for compacted history (complete=%s)', async (historyComplete) => {
    const messages: Message[] = [
      { id: 'compact', role: 'system', kind: 'compaction', content: 'summary', createdAt: 0 },
      { id: 'unknown', role: 'assistant', createdAt: 1, content: '[file](a.ts)' },
    ]
    await render(<ChatFileNavigation workingDir="/repo" enabled messages={messages} historyComplete={historyComplete}><MessageList messages={messages} /></ChatFileNavigation>)
    await click('button[title^="open a.ts"]')
    expect(opened).not.toHaveBeenCalled()
    expect(document.querySelector('[role="alertdialog"]')).not.toBeNull()
  })

  it('asks before resolving legacy messages and allows cancelling', async () => {
    const messages: Message[] = [{ id: 'legacy', role: 'assistant', createdAt: 0, content: '[file](a.ts)' }]
    await render(<ChatFileNavigation workingDir="/repo" enabled messages={messages}><MessageList messages={messages} /></ChatFileNavigation>)
    await click('button[title^="open a.ts"]')
    expect(opened).not.toHaveBeenCalled()
    expect(document.querySelector('[role="alertdialog"]')?.textContent).toContain('/repo/a.ts')
    const buttons = () => Array.from(document.querySelectorAll<HTMLButtonElement>('[role="alertdialog"] button'))
    await act(() => buttons().find((b) => b.textContent === 'Cancel')?.click())
    expect(opened).not.toHaveBeenCalled()
    await click('button[title^="open a.ts"]')
    await act(() => buttons().find((b) => b.textContent === 'Open file')?.click())
    expectFile('/repo/a.ts')
  })
  it('does not apply an old confirmation after a newer absolute link', async () => {
    const messages: Message[] = [{ id: 'legacy', role: 'assistant', createdAt: 0, content: '[old](a.ts) [new](/new/b.ts)' }]
    await render(<ChatFileNavigation workingDir="/repo" enabled messages={messages}><MessageList messages={messages} /></ChatFileNavigation>)
    await click('button[title^="open a.ts"]')
    await click('button[title^="open /new/b.ts"]')
    const accept = Array.from(document.querySelectorAll<HTMLButtonElement>('[role="alertdialog"] button')).find((b) => b.textContent === 'Open file')
    await act(async () => { accept?.click() })
    expectFile('/new/b.ts')
    expect(opened).toHaveBeenCalledTimes(1)
  })
})
