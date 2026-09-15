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

describe('ConversationRow touch actions', () => {
  it('exposes a named rename button without a double-click or keyboard', () => {
    const html = renderRow('Mobile chat')
    expect(html).toContain('aria-label="rename Mobile chat"')
    expect(html).toContain('title="Rename conversation"')
    expect(html).toContain('aria-label="delete Mobile chat"')
  })

  it('gives adjacent actions separate 48px coarse-pointer targets', () => {
    const html = renderRow('Mobile chat')
    expect(html.match(/pointer-coarse:min-h-12/g)).toHaveLength(2)
    expect(html.match(/pointer-coarse:min-w-12/g)).toHaveLength(2)
  })

  it('escapes conversation titles in action labels', () => {
    expect(renderRow('Chat "one"')).toContain(
      'aria-label="rename Chat &quot;one&quot;"',
    )
  })
})
