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

  it('exposes a named menu trigger and a pin shortcut', () => {
    const html = renderRow('Mobile chat')
    expect(html).toContain('aria-label="more actions for Mobile chat"')
    expect(html).toContain('aria-label="pin Mobile chat"')
    expect(html).not.toContain('aria-label="rename Mobile chat"')
    expect(html).not.toContain('aria-label="delete Mobile chat"')
  })

  it('keeps the pin shortcut off coarse pointers', () => {
    const html = renderRow('Mobile chat')
    const pin = html.slice(html.indexOf('aria-label="pin Mobile chat"') - 400)
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

  it('wraps the actions in the cluster that leaves the row flow', () => {
    /* The cluster is what lets a hidden action hold no width; loose actions
       under the trailing span reserve 26 px each and gutter the whole list. */
    const html = renderRow('Mobile chat')
    const meta = html.indexOf('class="iii-ui-tree-item__meta"')
    const cluster = html.indexOf('class="iii-ui-tree-item__actions"')
    const trigger = html.indexOf('aria-label="more actions for Mobile chat"')

    expect(cluster).toBeGreaterThan(meta)
    expect(cluster).toBeLessThan(trigger)
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
})
