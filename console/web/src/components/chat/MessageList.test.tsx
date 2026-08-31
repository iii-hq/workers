import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type {
  AssistantMessage,
  FunctionTriggerMessage,
  Message,
  ThoughtMessage,
  UserMessage,
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
  it('keeps lightweight call shells while mounting only the latest calls', () => {
    const html = renderToStaticMarkup(<MessageList messages={transcript()} />)

    expect(html.match(/data-message-role="function-call"/g)).toHaveLength(2)
    expect(
      html.match(/class="chat-activity-item" data-visible="true"/g),
    ).toHaveLength(2)
    expect(
      html.match(/class="chat-activity-item" data-visible="false"/g),
    ).toHaveLength(2)
    expect(html).toContain('3 triggers')
    expect(html).toContain('show all')
    expect(html).toContain('data-active="false" aria-hidden="true"')
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
    expect(
      html.match(/class="chat-activity-item" data-visible="false"/g),
    ).toBeNull()
    expect(html).toContain('show latest')
  })

  it('reveals a collapsed call targeted by an external landing', () => {
    const html = renderToStaticMarkup(
      <MessageList messages={transcript()} focusMessageId="c1" />,
    )

    // c1 hides behind the first group's collapse; the landing request must
    // expand that group in the same render so the row exists to center.
    expect(html.match(/data-message-role="function-call"/g)).toHaveLength(4)
    expect(
      html.match(/class="chat-activity-item" data-visible="false"/g),
    ).toBeNull()
    expect(html).toContain('data-message-row="c1"')
    expect(html).toContain('show latest')
  })

  it('keeps groups collapsed when the landing target is visible elsewhere', () => {
    const html = renderToStaticMarkup(
      <MessageList messages={transcript()} focusMessageId="intro" />,
    )

    expect(html.match(/data-message-role="function-call"/g)).toHaveLength(2)
    expect(
      html.match(/class="chat-activity-item" data-visible="false"/g),
    ).toHaveLength(2)
    expect(html).toContain('show all')
  })

  it('groups triggers separated only by completed, hidden thoughts', () => {
    const completedThought = (id: string): ThoughtMessage => ({
      id,
      role: 'thought',
      content: `reasoning ${id}`,
      durationMs: 100,
      streaming: false,
      createdAt: 0,
    })
    const html = renderToStaticMarkup(
      <MessageList
        transcriptHydrated={false}
        messages={[
          call('c1'),
          completedThought('t1'),
          call('c2'),
          call('c3'),
          completedThought('t2'),
          call('c4'),
        ]}
      />,
    )

    expect(html.match(/data-function-trigger-group=""/g)).toHaveLength(1)
    expect(html).toContain('data-function-trigger-count="4"')
    expect(html).toContain('4 triggers')
    expect(html).not.toContain('2 triggers')
  })

  it('reveals a hidden wake pair when the landing targets its notification', () => {
    const notification: UserMessage = {
      id: 'e_fire_sub_1_1',
      role: 'user',
      content: '[notification] build: {"ok":true}',
      createdAt: 0,
      notification: true,
    }
    const fired: Message = {
      id: 'e_trigfired_sub_1_1',
      role: 'system',
      kind: 'trigger-fired',
      content: 'build · notified this chat',
      trigger: {
        subscription_id: 'sub_1',
        target: 'harness::send',
        once: false,
        retired: false,
        fired_at: 1,
      },
      createdAt: 0,
    }
    const html = renderToStaticMarkup(
      <MessageList
        messages={[notification, fired, call('c1'), call('c2')]}
        focusMessageId="e_fire_sub_1_1"
      />,
    )

    // The pair collapses to one row carrying both entry ids; the absorbed
    // notification id must reveal it and be findable on the row.
    expect(html).toContain(
      'data-message-row="e_trigfired_sub_1_1 e_fire_sub_1_1"',
    )
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

  it('renders the branded waiting indicator while the model is pending', () => {
    const user: UserMessage = {
      id: 'user-1',
      role: 'user',
      content: 'Build the feature.',
      createdAt: 0,
    }
    const html = renderToStaticMarkup(
      <MessageList
        messages={[user]}
        isThinking
        thinkingDetail="dispatching model"
      />,
    )

    expect(html).toContain('data-model-waiting=""')
    expect(html).toContain('aria-label="dispatching model"')
    expect(html.match(/model-waiting-wordmark-segment/g)).toHaveLength(3)
  })
})
