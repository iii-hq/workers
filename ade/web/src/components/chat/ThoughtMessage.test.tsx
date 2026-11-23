import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { ThoughtMessage as ThoughtMessageType } from '@/types/chat'
import { ThoughtMessage, thoughtLabel } from './ThoughtMessage'

function thought(streaming: boolean, durationMs = 1250): ThoughtMessageType {
  return {
    id: 'thought-1',
    role: 'thought',
    content: 'Inspecting the message stream',
    durationMs: streaming ? 0 : durationMs,
    streaming,
    createdAt: 0,
  }
}

describe('ThoughtMessage', () => {
  it('renders the thought open while it is streaming', () => {
    const html = renderToStaticMarkup(
      <ThoughtMessage message={thought(true)} />,
    )

    expect(html).toContain('Thinking')
    expect(html).toContain('Inspecting the message stream')
    expect(html).toContain('data-state="streaming"')
    expect(html).toContain('<details class="iii-details group/thought" open')
    expect(html).toContain('chat-thought__viewport')
  })

  it('keeps a finished thought in the transcript, folded to its label', () => {
    const html = renderToStaticMarkup(
      <ThoughtMessage message={thought(false)} />,
    )

    expect(html).toContain('data-state="done"')
    expect(html).toContain('Thought for 1s')
    expect(html).toContain('Inspecting the message stream')
    expect(html).not.toContain(
      '<details class="iii-details group/thought" open',
    )
    expect(html).not.toContain('<div class="chat-thought" aria-hidden')
    expect(html).not.toContain('inert=""')
    expect(html).not.toContain('class="h-[10px] w-[5px]"')
  })

  it('labels a thought without a known duration plainly', () => {
    expect(thoughtLabel(thought(false, 0))).toBe('Thought')
    expect(thoughtLabel(thought(true))).toBe('Thinking')
  })
})
