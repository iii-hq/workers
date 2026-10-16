import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import type { Conversation } from '@/types/chat'
import { ConversationRow } from './ConversationRow'

/** Render an isolated conversation row for static accessibility assertions. */
function renderRow(title: string) {
  return renderToStaticMarkup(
    <ConversationRow
      conversation={{
        id: 'test',
        title,
        model: null,
        messages: [],
        createdAt: 1,
        updatedAt: 1,
      }}
      active={false}
      onSelect={vi.fn()}
      onRename={vi.fn()}
      onRemove={vi.fn()}
    />,
  )
}

describe('ConversationRow row actions', () => {
  it('exposes a named rename button without a double-click or keyboard', () => {
    const html = renderRow('Mobile chat')
    expect(html).toContain('aria-label="rename Mobile chat"')
    expect(html).toContain('title="Rename conversation"')
    expect(html).toContain('aria-label="delete Mobile chat"')
  })

  it('keeps renaming off coarse pointers', () => {
    /* Rename is a desktop affordance (double-click, F2, the pencil); a
       coarse pointer drops the pencil so touch rows keep one trailing
       action, which is what the tree recipe reserves room for. */
    const html = renderRow('Mobile chat')
    const rename = html.slice(
      html.indexOf('aria-label="rename Mobile chat"') - 300,
      html.indexOf('aria-label="rename Mobile chat"'),
    )
    expect(rename).toContain('data-pointer="fine"')
    expect(rename).not.toContain('pointer-coarse:min-h-12')
  })

  it('gives the surviving touch action a 48px coarse-pointer target', () => {
    const html = renderRow('Mobile chat')
    expect(html.match(/pointer-coarse:min-h-12/g)).toHaveLength(1)
    expect(html.match(/pointer-coarse:min-w-12/g)).toHaveLength(1)
  })

  it('wraps the actions in the cluster that leaves the row flow', () => {
    /* The cluster is what lets a hidden action hold no width; loose actions
       under the trailing span reserve 26 px each and gutter the whole list. */
    const html = renderRow('Mobile chat')
    const meta = html.indexOf('class="iii-ui-tree-item__meta"')
    const cluster = html.indexOf('class="iii-ui-tree-item__actions"')
    const rename = html.indexOf('aria-label="rename Mobile chat"')

    expect(cluster).toBeGreaterThan(meta)
    expect(cluster).toBeLessThan(rename)
  })

  it('puts the sub-agent caret last, so hover actions never slide into its place', () => {
    const row = (props: { hasChildren?: boolean; caretColumn?: boolean }) =>
      renderToStaticMarkup(
        <ConversationRow
          conversation={{
            id: 'p',
            title: 'Parent',
            model: null,
            messages: [],
            createdAt: 1,
            updatedAt: 1,
          }}
          active={false}
          onSelect={vi.fn()}
          onRename={vi.fn()}
          onRemove={vi.fn()}
          {...props}
        />,
      )

    const parent = row({ hasChildren: true, caretColumn: true })
    const caret = parent.indexOf('aria-label="collapse sub-agents"')
    expect(caret).toBeGreaterThan(parent.indexOf('aria-label="delete Parent"'))
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
      'aria-label="rename Chat &quot;one&quot;"',
    )
  })
})

/** Render a row with its glyph inputs: profile, origin and spawn appearance. */
function renderGlyphRow(
  conversation: Partial<Conversation>,
  depth: number,
): string {
  return renderToStaticMarkup(
    <ConversationRow
      conversation={{
        id: 'glyph',
        title: 'KAN-21 tests',
        model: null,
        messages: [],
        createdAt: 1,
        updatedAt: 1,
        ...conversation,
      }}
      depth={depth}
      active={false}
      onSelect={vi.fn()}
      onRename={vi.fn()}
      onRemove={vi.fn()}
    />,
  )
}

const IMPL_PROFILE = {
  id: 'deepwork-impl',
  name: 'Deepwork Implementer',
  icon: 'code',
} as const

describe('ConversationRow glyph', () => {
  it("shows a sub-agent's spawn icon over its profile's", () => {
    /* Two phases on one profile (tests and code on deepwork-impl) differ
       only by the icon their spawn picked. */
    const html = renderGlyphRow(
      {
        agentProfile: IMPL_PROFILE,
        spawnedBy: 'agent',
        subagentAppearance: {
          name: 'KAN-21 tests',
          icon: 'test',
          color: 'amber',
        },
      },
      2,
    )
    expect(html).toContain('lucide-flask-conical')
    expect(html).not.toContain('lucide-code')
    expect(html).toContain('title="Deepwork Implementer"')
  })

  it("keeps the profile's icon when the spawn picked none", () => {
    const html = renderGlyphRow(
      {
        agentProfile: IMPL_PROFILE,
        spawnedBy: 'agent',
        subagentAppearance: { name: 'KAN-21 code' },
      },
      2,
    )
    expect(html).toContain('lucide-code')
  })

  it("keeps the profile's icon on a root session", () => {
    const html = renderGlyphRow(
      {
        agentProfile: IMPL_PROFILE,
        spawnedBy: 'agent',
        subagentAppearance: { name: 'KAN-21 tests', icon: 'test' },
      },
      0,
    )
    expect(html).toContain('lucide-code')
    expect(html).not.toContain('lucide-flask-conical')
  })
})
