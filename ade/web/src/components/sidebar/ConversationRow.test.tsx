import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
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

  it('escapes conversation titles in action labels', () => {
    expect(renderRow('Chat "one"')).toContain(
      'aria-label="rename Chat &quot;one&quot;"',
    )
  })
})
