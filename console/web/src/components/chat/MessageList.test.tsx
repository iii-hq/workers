import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type {
  AssistantMessage,
  FunctionTriggerMessage,
  Message,
} from '@/types/chat'
import { MessageList } from './MessageList'

function call(id: string): FunctionTriggerMessage {
  return {
    id,
    role: 'function-trigger',
    functionId: 'shell::run',
    description: `call ${id}`,
    input: { command: id },
    output: { ok: true },
    createdAt: 0,
  }
}

function assistant(
  id: string,
  content: string,
  stopReason: AssistantMessage['stopReason'],
): AssistantMessage {
  return {
    id,
    role: 'assistant',
    content,
    stopReason,
    createdAt: 0,
  }
}

function transcript(): Message[] {
  return [
    assistant('intro', 'I will inspect the implementation.', 'function_call'),
    call('c1'),
    call('c2'),
    call('c3'),
    assistant(
      'progress',
      'The implementation uses one shared mapper; next I will update it.',
      'function_call',
    ),
    call('c4'),
    assistant('final', 'The update is complete.', 'end'),
  ]
}

describe('MessageList function-trigger groups', () => {
  it('renders only the latest call of a collapsed sequence and its summary', () => {
    const html = renderToStaticMarkup(<MessageList messages={transcript()} />)

    expect(html.match(/data-message-role="function-call"/g)).toHaveLength(2)
    expect(html).toContain('3 triggers')
    expect(html).toContain('show all')
    expect(html).toContain(
      'The implementation uses one shared mapper; next I will update it.',
    )
    expect(html).toContain('The update is complete.')
  })

  it('renders every call when groups default to expanded', () => {
    const html = renderToStaticMarkup(
      <MessageList messages={transcript()} defaultOpenCalls />,
    )

    expect(html.match(/data-message-role="function-call"/g)).toHaveLength(4)
    expect(html).toContain('show latest')
  })

  it('exposes a pending approval as a focusable, named action target', () => {
    const pending: FunctionTriggerMessage = {
      ...call('approval'),
      pendingApproval: true,
      sessionId: 'session-1',
      functionTriggerId: 'function-call-1',
    }
    const html = renderToStaticMarkup(
      <MessageList messages={[pending]} onResolveApproval={async () => {}} />,
    )

    expect(html).toContain('data-message-id="approval"')
    expect(html).toContain('aria-label="action required for shell::run"')
    expect(html).toContain('tabindex="-1"')
    expect(html).toContain('data-approval-actions=""')
  })

  it('anchors user and assistant messages by id for the turn rail', () => {
    const html = renderToStaticMarkup(
      <MessageList
        messages={[
          { id: 'u-1', createdAt: 1, role: 'user', content: 'hello' },
          { id: 'a-1', createdAt: 2, role: 'assistant', content: 'hi' },
        ]}
      />,
    )

    expect(html).toContain('data-message-role="user" data-message-id="u-1"')
    expect(html).toContain(
      'data-message-role="assistant" data-message-id="a-1"',
    )
  })
})
