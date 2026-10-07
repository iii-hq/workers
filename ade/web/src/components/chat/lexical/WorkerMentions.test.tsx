// @vitest-environment jsdom

import {
  $createParagraphNode,
  $createTextNode,
  $getRoot,
  type LexicalEditor,
} from 'lexical'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { Markdown } from '@/lib/markdown'
import {
  iiiMentionRuntime,
  type MentionRuntime,
  setMentionRuntime,
} from '@/lib/mentions/runtime'
import type { MentionProvider } from '@/lib/mentions/types'
import { LexicalShell } from '../LexicalShell'
import { $exportComposerMarkdown } from './composer-markdown'

vi.mock('@/lib/iii-client', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  // The / palette fetches skills when it opens; keep that fetch pending.
  getIiiClient: () => new Promise(() => {}),
}))

const kanban: MentionProvider = {
  v: 1,
  name: 'kanban',
  label: 'Tickets',
  icon: 'ticket',
  color: 'blue',
  search: 'kanban::mention::search',
  getFunctionId: 'kanban::mention::get',
}

const TICKET = '6ac4f6df-ec84-83e9-b480-4b54b9931ce0'

let container: HTMLDivElement
let root: Root
let runtime: MentionRuntime & {
  search: ReturnType<typeof vi.fn>
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  if (!('getBoundingClientRect' in Range.prototype)) {
    Object.defineProperty(Range.prototype, 'getBoundingClientRect', {
      configurable: true,
      value: () => ({
        top: 0,
        left: 0,
        right: 0,
        bottom: 0,
        width: 0,
        height: 0,
        x: 0,
        y: 0,
      }),
    })
  }
  if (!('ResizeObserver' in globalThis)) {
    vi.stubGlobal(
      'ResizeObserver',
      class {
        observe() {}
        unobserve() {}
        disconnect() {}
      },
    )
  }
  runtime = {
    listProviders: vi.fn(async () => [kanban]),
    search: vi.fn(
      async (_provider: MentionProvider, { query }: { query: string }) =>
        query === 'nothing'
          ? []
          : [{ id: TICKET, label: 'Fix login redirect', hint: 'KAN-12' }],
    ),
    get: vi.fn(async () => ({
      id: TICKET,
      label: 'Fix login redirect',
      hint: 'KAN-12',
    })),
  }
  setMentionRuntime(runtime)
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
  setMentionRuntime(iiiMentionRuntime)
  vi.unstubAllGlobals()
})

async function wait(ms: number) {
  await act(async () => {
    await new Promise<void>((resolve) => setTimeout(resolve, ms))
  })
}

async function renderComposer(text: string) {
  const onSubmit = vi.fn()
  let editor!: LexicalEditor
  await act(async () => {
    root.render(
      <LexicalShell
        clearToken={0}
        onChange={() => {}}
        onSubmit={onSubmit}
        initialContent={(instance) => {
          editor = instance
          const paragraph = $createParagraphNode()
          paragraph.append($createTextNode(text))
          $getRoot().append(paragraph)
        }}
      />,
    )
  })
  await act(async () => {
    editor.update(() => $getRoot().selectEnd(), { discrete: true })
    await new Promise<void>((resolve) => setTimeout(resolve, 0))
  })
  const editable = container.querySelector<HTMLElement>(
    '[aria-label="message composer"]',
  )
  if (!editable) throw new Error('Composer not mounted')
  // Providers load, the menu opens, a debounced search lands.
  await wait(250)
  return { editor, editable, onSubmit }
}

async function press(editable: HTMLElement, key: 'Tab' | 'Enter') {
  const event = new KeyboardEvent('keydown', {
    key,
    code: key,
    keyCode: key === 'Tab' ? 9 : 13,
    bubbles: true,
    cancelable: true,
  })
  await act(async () => {
    editable.dispatchEvent(event)
  })
  await wait(250)
  return event
}

function markdown(editor: LexicalEditor): string {
  return editor.getEditorState().read(() => $exportComposerMarkdown())
}

/** Every option on screen (Lexical also leaves an empty anchor listbox). */
function menuText(): string {
  return [...document.querySelectorAll('[role="option"]')]
    .map((option) => option.textContent)
    .join('\n')
}

describe('worker mentions in the composer', () => {
  it('offers the provider for @kan and drills into its search on Tab', async () => {
    const { editor, editable } = await renderComposer('@kan')
    expect(menuText()).toContain('@kanban')

    await press(editable, 'Tab')
    expect(markdown(editor)).toBe('@kanban:')
    expect(runtime.search).toHaveBeenCalledWith(
      kanban,
      expect.objectContaining({ query: '' }),
    )
    expect(menuText()).toContain('Fix login redirect')

    await press(editable, 'Enter')
    expect(markdown(editor)).toBe(`@kanban(id="${TICKET}") `)
    const pill = container.querySelector('[data-worker-mention="kanban"]')
    expect(pill?.getAttribute('data-mention-id')).toBe(TICKET)
    expect(pill?.textContent).toContain('Fix login redirect')
  })

  it('groups provider results under @text and inserts a picked item', async () => {
    const { editor, editable } = await renderComposer('see @fix')
    const headers = [...document.querySelectorAll('[role="presentation"]')].map(
      (header) => header.textContent,
    )
    expect(headers).toContain('Tickets')
    expect(menuText()).toContain('KAN-12')
    await press(editable, 'Enter')
    expect(markdown(editor)).toBe(`see @kanban(id="${TICKET}") `)
  })

  it('does not send a half-typed scoped search on Enter', async () => {
    const { editor, editable, onSubmit } =
      await renderComposer('@kanban:nothing')
    await press(editable, 'Enter')
    expect(onSubmit).not.toHaveBeenCalled()
    expect(markdown(editor)).toBe('@kanban:nothing')
  })

  it('turns a pasted token into the pill', async () => {
    const { editor } = await renderComposer(`ping @kanban(id="${TICKET}") now`)
    expect(markdown(editor)).toBe(`ping @kanban(id="${TICKET}") now`)
    expect(
      container.querySelector('[data-worker-mention="kanban"]'),
    ).not.toBeNull()
  })
})

describe('worker mentions in rendered messages', () => {
  it('renders the token as a pill and leaves code alone', () => {
    const html = renderToStaticMarkup(
      <Markdown>{`see @kanban(id="${TICKET}") and \`@kanban(id="x")\``}</Markdown>,
    )
    expect(html).toContain(`data-mention-id="${TICKET}"`)
    expect(html).toContain('<code')
    expect(html).toContain('@kanban(id=&quot;x&quot;)')
    expect(html.match(/data-worker-mention=/g)).toHaveLength(1)
  })

  it('still renders a reply whose model dropped the closing quote', () => {
    const html = renderToStaticMarkup(
      <Markdown>{`In @session(id="console-9db3a777), titled "hello":`}</Markdown>,
    )
    expect(html).toContain('data-mention-id="console-9db3a777"')
    // The `)` belongs to the token: nothing of it is left in the prose.
    expect(html).toContain('</span>, titled')
  })
})
