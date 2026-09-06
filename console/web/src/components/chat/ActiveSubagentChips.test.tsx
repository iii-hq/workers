import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import type { Conversation } from '@/types/chat'
import { ActiveSubagentChips } from './ActiveSubagentChips'

function conversation(
  id: string,
  overrides: Partial<Conversation> = {},
): Conversation {
  return {
    id,
    title: id,
    model: null,
    messages: [],
    createdAt: 1,
    updatedAt: 1,
    ...overrides,
  }
}

describe('ActiveSubagentChips', () => {
  it('renders named, colored and accessible active child controls', () => {
    const html = renderToStaticMarkup(
      <ActiveSubagentChips
        conversations={[
          conversation('frontend', {
            parentId: 'root',
            status: 'working',
            subagentAppearance: {
              name: 'Frontend',
              icon: 'code',
              color: 'purple',
            },
          }),
          conversation('done', {
            parentId: 'root',
            status: 'done',
          }),
        ]}
        connectionState="connected"
        onOpen={vi.fn()}
        rootSessionId="root"
      />,
    )

    expect(html).toContain('data-active-subagent-chips=""')
    expect(html).toContain('data-session-id="frontend"')
    expect(html).toContain('data-color="purple"')
    expect(html).toContain('data-status="working"')
    expect(html).toContain('lucide-code-xml')
    expect(html).toContain('width="16"')
    expect(html).toContain(
      'aria-label="Open Frontend sub-agent in a new panel (working)"',
    )
    expect(html).not.toContain('data-subagent-terminal-summary=""')
    expect(html).not.toContain('1 done')
  })

  it('keeps the terminal summary when a child failed or stopped', () => {
    const html = renderToStaticMarkup(
      <ActiveSubagentChips
        conversations={[
          conversation('completed', {
            parentId: 'root',
            status: 'done',
          }),
          conversation('failed', {
            parentId: 'root',
            status: 'error',
          }),
          conversation('stopped', {
            parentId: 'root',
            status: 'done',
            statusReason: 'stopped by user',
          }),
        ]}
        connectionState="connected"
        onOpen={vi.fn()}
        rootSessionId="root"
      />,
    )

    expect(html).toContain('data-subagent-terminal-summary=""')
    expect(html).toContain('1 done')
    expect(html).toContain('1 failed')
    expect(html).toContain('1 stopped')
  })

  it('renders a disconnected active state and bounds visible controls', () => {
    const html = renderToStaticMarkup(
      <ActiveSubagentChips
        conversations={[
          conversation('one', { parentId: 'root', status: 'working' }),
          conversation('two', { parentId: 'root', status: 'working' }),
        ]}
        connectionState="reconnecting"
        maxVisible={1}
        onOpen={vi.fn()}
        rootSessionId="root"
      />,
    )

    expect(html.match(/<button/g)).toHaveLength(1)
    expect(html).toContain('data-status="disconnected"')
    expect(html).toContain('+1 active')
  })

  it('keeps every normally bounded active child directly actionable', () => {
    const html = renderToStaticMarkup(
      <ActiveSubagentChips
        conversations={Array.from({ length: 12 }, (_, index) =>
          conversation(`child-${index}`, {
            parentId: 'root',
            status: 'working',
          }),
        )}
        connectionState="connected"
        onOpen={vi.fn()}
        rootSessionId="root"
      />,
    )

    expect(html.match(/<button/g)).toHaveLength(12)
    expect(html).not.toContain('active</span>')
  })

  it('stays absent when the root has no descendant sessions', () => {
    const html = renderToStaticMarkup(
      <ActiveSubagentChips
        conversations={[
          conversation('unrelated', { parentId: 'another-root' }),
        ]}
        connectionState="connected"
        onOpen={vi.fn()}
        rootSessionId="root"
      />,
    )

    expect(html).toBe('')
  })

  it('stays absent when every descendant session completed', () => {
    const html = renderToStaticMarkup(
      <ActiveSubagentChips
        conversations={[
          conversation('done', {
            parentId: 'root',
            status: 'done',
          }),
        ]}
        connectionState="connected"
        onOpen={vi.fn()}
        rootSessionId="root"
      />,
    )

    expect(html).toBe('')
  })

  it('keeps the truncation notice when the completed list was limited', () => {
    const html = renderToStaticMarkup(
      <ActiveSubagentChips
        conversations={Array.from({ length: 513 }, (_, index) =>
          conversation(`done-${index}`, {
            parentId: 'root',
            status: 'done',
            createdAt: index,
          }),
        )}
        connectionState="connected"
        onOpen={vi.fn()}
        rootSessionId="root"
      />,
    )

    expect(html).toContain('Sub-agent list limited')
    expect(html).not.toContain('data-subagent-terminal-summary=""')
  })
})
