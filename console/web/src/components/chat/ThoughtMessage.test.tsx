import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { ThoughtMessage as ThoughtMessageType } from '@/types/chat'
import { ThoughtMessage } from './ThoughtMessage'

function thought(streaming: boolean): ThoughtMessageType {
  return {
    id: 'thought-1',
    role: 'thought',
    content: 'Inspecting the message stream',
    durationMs: streaming ? 0 : 1250,
    streaming,
    createdAt: 0,
  }
}

describe('ThoughtMessage', () => {
  it('renders the thought while it is streaming', () => {
    const html = renderToStaticMarkup(
      <ThoughtMessage message={thought(true)} />,
    )

    expect(html).toContain('Thought…')
    expect(html).toContain('Inspecting the message stream')
    expect(html).toContain('data-state="streaming"')
    expect(html).toContain('chat-thought__viewport')
  })

  it('removes the thought subtree after streaming completes', () => {
    expect(
      renderToStaticMarkup(<ThoughtMessage message={thought(false)} />),
    ).toBe('')
  })

  it('exposes an explicit settling state to a parent presence manager', () => {
    const html = renderToStaticMarkup(
      <ThoughtMessage message={thought(false)} settling />,
    )

    expect(html).toContain('data-state="settling"')
    expect(html).toContain('aria-hidden="true"')
    expect(html).toContain('inert=""')
    expect(html).toContain('Inspecting the message stream')
    expect(html).not.toContain('class="h-[10px] w-[5px]"')
  })
})
