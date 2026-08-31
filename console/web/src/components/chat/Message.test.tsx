import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { AssistantMessage, UserMessage } from '@/types/chat'
import { Message, shouldAnimateStreamingBlockBurst } from './Message'

const spawnMessage: UserMessage = {
  id: 'spawn-task',
  role: 'user',
  content: 'Compare the available rendering approaches.',
  spawn: true,
  createdAt: 0,
}

const assistantMessage: AssistantMessage = {
  id: 'assistant-message',
  role: 'assistant',
  content: 'Done.',
  model: 'codex/gpt-5.6-sol',
  createdAt: 0,
}

describe('AssistantMessage', () => {
  it('shows the session agent profile name in the message header', () => {
    const html = renderToStaticMarkup(
      <Message message={assistantMessage} agentName="Researcher" />,
    )

    expect(html).toContain('>Researcher</span>')
    expect(html).toContain('· codex/gpt-5.6-sol')
  })

  it('marks only a live assistant surface for its first content reveal', () => {
    const streaming = renderToStaticMarkup(
      <Message message={{ ...assistantMessage, streaming: true }} />,
    )
    const settled = renderToStaticMarkup(<Message message={assistantMessage} />)

    expect(streaming).toContain('data-assistant-stream-surface=""')
    expect(streaming).toContain('class="assistant-stream-surface is-entering"')
    expect(settled).toContain('data-assistant-stream-surface=""')
    expect(settled).not.toContain('class="assistant-stream-surface')
  })

  it('keeps rich Markdown intact instead of wrapping streamed words', () => {
    const html = renderToStaticMarkup(
      <Message
        message={{
          ...assistantMessage,
          streaming: true,
          content: '**Ready.**\n\n- first\n- second',
        }}
      />,
    )

    expect(html).toContain('<strong')
    expect(html).toContain('<ul')
    expect(html).not.toContain('t-stream-w')
  })

  it('skips motion for large block bursts and reduced-motion users', () => {
    expect(shouldAnimateStreamingBlockBurst(1, 120, false)).toBe(true)
    expect(shouldAnimateStreamingBlockBurst(4, 120, false)).toBe(false)
    expect(shouldAnimateStreamingBlockBurst(1, 481, false)).toBe(false)
    expect(shouldAnimateStreamingBlockBurst(1, 120, true)).toBe(false)

    const snapshot = renderToStaticMarkup(
      <Message
        message={{
          ...assistantMessage,
          streaming: true,
          content: 'x'.repeat(481),
        }}
      />,
    )
    expect(snapshot).not.toContain('class="assistant-stream-surface')
  })
})

describe('SpawnTaskMessage', () => {
  it('uses the shared card recipe and current sub-agent identity', () => {
    const html = renderToStaticMarkup(
      <Message
        message={spawnMessage}
        spawnContext={{
          title: 'Researcher',
          model: 'codex/gpt-5.6-luna',
          appearance: {
            name: 'Researcher',
            icon: 'search',
            color: 'purple',
          },
        }}
      />,
    )

    expect(html).toContain('data-message-role="spawn-task"')
    expect(html).toContain('class="iii-ui-card"')
    expect(html).toContain('iii-ui-card__header')
    expect(html).toContain('iii-ui-card__body')
    expect(html).toContain('Researcher')
    expect(html).toContain('codex/gpt-5.6-luna')
    expect(html).toContain('data-tone="accent"')
    expect(html).toContain('Compare the available rendering approaches.')
  })

  it('keeps historical spawn entries readable without session metadata', () => {
    const html = renderToStaticMarkup(<Message message={spawnMessage} />)

    expect(html).toContain('aria-label="Sub-agent spawn task"')
    expect(html).toContain('Compare the available rendering approaches.')
  })
})
