// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { Conversation } from '@/types/chat'
import { ConversationRow } from './ConversationRow'

function conversation(title: string, extra: Partial<Conversation> = {}) {
  return {
    id: 'test',
    title,
    model: null,
    messages: [],
    createdAt: 1,
    updatedAt: 1,
    ...extra,
  } satisfies Conversation
}

/** Render an isolated conversation row for static accessibility assertions. */
function renderRow(title: string, extra: Partial<Conversation> = {}) {
  return renderToStaticMarkup(
    <ConversationRow
      conversation={conversation(title, extra)}
      active={false}
      onSelect={vi.fn()}
      onRename={vi.fn()}
      onRemove={vi.fn()}
      onTogglePin={vi.fn()}
    />,
  )
}

describe('ConversationRow row actions', () => {
  it('opens with the title: a plain chat carries no leading dot or mark', () => {
    const html = renderRow('Mobile chat')
    expect(html).not.toContain('iii-ui-tree-item__icon')
    expect(html.indexOf('Mobile chat')).toBeGreaterThan(0)
  })

  it('leads the title with the working dot while the agent generates', () => {
    const html = renderRow('Hotel queue', { status: 'working' })
    const dot = html.indexOf('conversation-row__status')
    expect(dot).toBeGreaterThan(0)
    expect(dot).toBeLessThan(html.indexOf('Hotel queue</'))
    expect(html).toContain('title="working"')
  })

  it('marks a chat waiting on the reader’s answer to its question', () => {
    const html = renderRow('Pick a stack', {
      status: 'done',
      statusReason: 'awaiting answer',
    })
    const mark = html.indexOf('data-awaiting-answer')
    expect(mark).toBeGreaterThan(0)
    expect(mark).toBeLessThan(html.indexOf('Pick a stack</'))
    expect(html).toContain('bg-warn')
    expect(html).toContain('waiting for your answer')
  })

  it('draws no waiting mark once a chat is just done', () => {
    const html = renderRow('Shipped', { status: 'done' })
    expect(html).not.toContain('data-awaiting-answer')
    expect(html).not.toContain('conversation-row__status')
  })

  it('exposes a named menu trigger and a pin shortcut', () => {
    const html = renderRow('Mobile chat')
    expect(html).toContain('aria-label="more actions for Mobile chat"')
    expect(html).toContain('aria-label="pin Mobile chat"')
    expect(html).not.toContain('aria-label="rename Mobile chat"')
    expect(html).not.toContain('aria-label="delete Mobile chat"')
  })

  it('keeps the pin shortcut off coarse pointers', () => {
    const html = renderRow('Mobile chat')
    const label = html.indexOf('aria-label="pin Mobile chat"')
    const pin = html.slice(html.lastIndexOf('<button', label), label)
    expect(pin).toContain('data-pointer="fine"')
    expect(pin).not.toContain('pointer-coarse:min-h-12')
  })

  it('gives the surviving touch action a 48px coarse-pointer target', () => {
    const html = renderRow('Mobile chat')
    expect(html.match(/pointer-coarse:min-h-12/g)).toHaveLength(1)
    expect(html.match(/pointer-coarse:min-w-12/g)).toHaveLength(1)
  })

  it('shows the pin at rest on a pinned chat and offers to unpin it', () => {
    const html = renderToStaticMarkup(
      <ConversationRow
        conversation={conversation('Pinned chat')}
        active={false}
        onSelect={vi.fn()}
        onRename={vi.fn()}
        onRemove={vi.fn()}
        pinned
        onTogglePin={vi.fn()}
      />,
    )
    expect(html).toContain('data-pinned="true"')
    expect(html).toContain('data-pinned-mark=""')
    expect(html).toContain('aria-label="unpin Pinned chat"')
  })

  it('keeps the pin left of the cell the time and the ⋯ share', () => {
    /* The ⋯ takes the time's place at the far right on hover; the pin has
       its own place before that cell, so it never moves under the pointer. */
    const html = renderToStaticMarkup(
      <ConversationRow
        conversation={conversation('Pinned chat')}
        active={false}
        onSelect={vi.fn()}
        onRename={vi.fn()}
        onRemove={vi.fn()}
        pinned
        onTogglePin={vi.fn()}
      />,
    )
    const pin = html.indexOf('aria-label="unpin Pinned chat"')
    const slot = html.indexOf('class="conversation-row__slot"')
    const meta = html.indexOf('class="iii-ui-tree-item__meta"')
    const trigger = html.indexOf('aria-label="more actions for Pinned chat"')

    expect(pin).toBeGreaterThan(-1)
    expect(slot).toBeGreaterThan(pin)
    expect(meta).toBeGreaterThan(slot)
    expect(trigger).toBeGreaterThan(meta)
  })

  it('is draggable only when it can be rearranged', () => {
    const reorder = {
      dragging: false,
      dropEdge: 'before' as const,
      onDragStart: vi.fn(),
      onDragOver: vi.fn(),
      onDrop: vi.fn(),
      onDragEnd: vi.fn(),
      onMove: vi.fn(),
      canMoveUp: true,
      canMoveDown: false,
    }
    const row = (withReorder: boolean) =>
      renderToStaticMarkup(
        <ConversationRow
          conversation={conversation('Pinned chat')}
          active={false}
          onSelect={vi.fn()}
          onRename={vi.fn()}
          onRemove={vi.fn()}
          pinned
          onTogglePin={vi.fn()}
          reorder={withReorder ? reorder : undefined}
        />,
      )
    expect(row(true)).toContain('draggable="true"')
    expect(row(true)).toContain('data-drop="before"')
    expect(row(false)).not.toContain('draggable')
  })

  it('puts the sub-agent caret last, so hover actions never slide into its place', () => {
    const row = (props: { hasChildren?: boolean; caretColumn?: boolean }) =>
      renderToStaticMarkup(
        <ConversationRow
          conversation={conversation('Parent', { id: 'p' })}
          active={false}
          onSelect={vi.fn()}
          onRename={vi.fn()}
          onRemove={vi.fn()}
          {...props}
        />,
      )

    const parent = row({ hasChildren: true, caretColumn: true })
    const caret = parent.indexOf('aria-label="collapse sub-agents"')
    expect(caret).toBeGreaterThan(
      parent.indexOf('aria-label="more actions for Parent"'),
    )
    expect(caret).toBeGreaterThan(
      parent.indexOf('class="iii-ui-tree-item__meta"'),
    )

    // A leaf keeps the caret's column empty only when the list has carets.
    expect(row({ caretColumn: true })).toContain(
      '<span aria-hidden="true" class="iii-ui-tree-item__caret" data-slot=""></span>',
    )
    expect(row({})).not.toContain('iii-ui-tree-item__caret')
  })

  it('escapes conversation titles in action labels', () => {
    expect(renderRow('Chat "one"')).toContain(
      'aria-label="more actions for Chat &quot;one&quot;"',
    )
  })

  it('keeps the age of an old chat one short unit wide', () => {
    const day = 86_400_000
    const age = (days: number) => {
      const html = renderRow('Old', { updatedAt: Date.now() - days * day })
      return html.match(/iii-ui-tree-item__meta[^>]*>([^<]+)</)?.[1]
    }
    expect(age(3)).toBe('3d')
    expect(age(20)).toBe('2w')
    expect(age(143)).toBe('4mo')
    expect(age(400)).toBe('1y')
  })
})

describe('ConversationRow menu', () => {
  let root: Root | null = null

  afterEach(() => {
    act(() => root?.unmount())
    root = null
    document.body.innerHTML = ''
    vi.unstubAllGlobals()
  })

  function mount(props: {
    onRemove: () => void
    onTogglePin: () => void
    onSelect: () => void
    pinned?: boolean
    reorder?: Parameters<typeof ConversationRow>[0]['reorder']
  }) {
    vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
    root = createRoot(document.body.appendChild(document.createElement('div')))
    act(() =>
      root?.render(
        <ConversationRow
          conversation={conversation('Mobile chat')}
          active={false}
          onRename={vi.fn()}
          {...props}
        />,
      ),
    )
  }

  const button = (label: string) =>
    [...document.querySelectorAll('button')].find(
      (el) => el.getAttribute('aria-label') === label,
    ) as HTMLButtonElement

  const open = () => {
    const trigger = button('more actions for Mobile chat')
    act(() => {
      trigger.dispatchEvent(
        new PointerEvent('pointerdown', { bubbles: true, button: 0 }),
      )
      trigger.click()
    })
  }

  const items = () =>
    [...document.querySelectorAll('[data-slot="dropdown-menu-item"]')].map(
      (el) => el.textContent?.trim(),
    )

  it('lists Rename, Pin and Delete, each with an icon', () => {
    const onSelect = vi.fn()
    mount({ onRemove: vi.fn(), onTogglePin: vi.fn(), onSelect })
    open()
    expect(items()).toEqual(['Rename', 'Pin', 'Delete'])
    for (const item of document.querySelectorAll(
      '[data-slot="dropdown-menu-item"]',
    )) {
      expect(item.querySelector('svg')).not.toBeNull()
    }
    expect(
      document
        .querySelector(
          '[data-slot="dropdown-menu-item"][data-variant="destructive"]',
        )
        ?.textContent?.trim(),
    ).toBe('Delete')
    // Opening the menu is not opening the chat.
    expect(onSelect).not.toHaveBeenCalled()
  })

  it('opens the chat from a tap on its time, which touch keeps on show', () => {
    const onSelect = vi.fn()
    mount({ onRemove: vi.fn(), onTogglePin: vi.fn(), onSelect })
    act(() =>
      (
        document.querySelector('.iii-ui-tree-item__meta') as HTMLElement
      ).click(),
    )
    expect(onSelect).toHaveBeenCalledTimes(1)
  })

  it('offers Unpin on a pinned chat and routes Delete to onRemove', () => {
    const onRemove = vi.fn()
    const onTogglePin = vi.fn()
    mount({ onRemove, onTogglePin, onSelect: vi.fn(), pinned: true })
    open()
    expect(items()).toEqual(['Rename', 'Unpin', 'Delete'])
    const del = [
      ...document.querySelectorAll<HTMLElement>(
        '[data-slot="dropdown-menu-item"]',
      ),
    ].find((el) => el.textContent?.trim() === 'Delete') as HTMLElement
    act(() => del.click())
    expect(onRemove).toHaveBeenCalledTimes(1)
  })

  it('offers Move up and Move down for a pinned chat that can be rearranged', () => {
    const onMove = vi.fn()
    mount({
      onRemove: vi.fn(),
      onTogglePin: vi.fn(),
      onSelect: vi.fn(),
      pinned: true,
      reorder: {
        dragging: false,
        onDragStart: vi.fn(),
        onDragOver: vi.fn(),
        onDrop: vi.fn(),
        onDragEnd: vi.fn(),
        onMove,
        canMoveUp: true,
        canMoveDown: false,
      },
    })
    open()
    expect(items()).toEqual(expect.arrayContaining(['Move up', 'Move down']))
    const item = (text: string) =>
      [
        ...document.querySelectorAll<HTMLElement>(
          '[data-slot="dropdown-menu-item"]',
        ),
      ].find((el) => el.textContent?.trim() === text)
    expect(item('Move down')?.hasAttribute('data-disabled')).toBe(true)
    act(() => item('Move up')?.click())
    expect(onMove).toHaveBeenCalledWith(-1)
  })
})
