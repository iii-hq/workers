import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type {
  AssistantMessage,
  FunctionTriggerMessage,
  Message,
  ThoughtMessage,
  UserMessage,
} from '@/types/chat'
import { liveAppendedKeys, MessageList, unloadedEntryIds } from './MessageList'

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

function triggerRegistration(id: string): FunctionTriggerMessage {
  return {
    id,
    role: 'function-trigger',
    functionId: 'engine::register_trigger',
    input: {
      trigger_type: 'state',
      config: { key: 'build', scope: 'ops' },
    },
    output: { subscription_id: `subscription-${id}` },
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
  it('keeps trigger registration visible with its antenna marker when collapsed', () => {
    const html = renderToStaticMarkup(
      <MessageList
        messages={[triggerRegistration('registration'), call('c1')]}
      />,
    )

    expect(html).toContain('data-message-row="registration"')
    expect(html).toContain('data-timeline-activity-kind="trigger-registration"')
    expect(html).toContain('lucide-radio-tower')
    expect(html).toContain('stroke-ok')
    expect(html).toContain('data-timeline-activity-kind="function"')
    expect(html).not.toContain('data-function-display-slot=""')
  })

  it('keeps lightweight call shells while mounting only the latest calls', () => {
    const html = renderToStaticMarkup(<MessageList messages={transcript()} />)

    expect(html.match(/data-message-role="function-call"/g)).toHaveLength(2)
    expect(
      html.match(/class="chat-activity-item" data-visible="true"/g),
    ).toHaveLength(2)
    expect(
      html.match(/class="chat-activity-item" data-visible="false"/g),
    ).toHaveLength(2)
    expect(html).toContain('Agent triggered 3 functions')
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
    expect(html).toContain('Agent triggered 4 functions')
    expect(html).not.toContain('Agent triggered 2 functions')
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

/* Only rows appended since the last commit get the live appear animation.
   Rows inserted INTO history (an older page prepended, a placeholder swapped
   for its whole entry) are old news and must not look like fresh messages. */
describe('liveAppendedKeys', () => {
  it('animates rows appended after the newest known row', () => {
    expect([...liveAppendedKeys(['a', 'b'], ['a', 'b', 'c', 'd'])]).toEqual([
      'c',
      'd',
    ])
  })

  it('never animates a page prepended above the known rows', () => {
    expect([
      ...liveAppendedKeys(['m', 'n'], ['h1', 'h2', 'h3', 'm', 'n']),
    ]).toEqual([])
  })

  it('ignores rows re-keyed inside history while still animating the tail', () => {
    // `a:1` replaced `a:0` when its whole entry landed; `z` arrived live.
    expect([
      ...liveAppendedKeys(['a:0', 'b', 'c'], ['a:1', 'b', 'c', 'z']),
    ]).toEqual(['z'])
  })

  it('anchors on the newest known row that survived', () => {
    // The optimistic last row was replaced by its durable id: that
    // replacement is the live one, not the older rows around it.
    expect([
      ...liveAppendedKeys(['a', 'e_idem_1'], ['a', 'e_durable_1']),
    ]).toEqual(['e_durable_1'])
    // With no known row left, everything new is live (first live message).
    expect([...liveAppendedKeys([], ['x'])]).toEqual(['x'])
  })
})

/* The loaded window's top edge: the row above the first message says where
   the history stands, and placeholders from a paged read still count in
   the group's disclosure and render as skeletons rather than empty panes. */
describe('MessageList paged history', () => {
  function unloadedCall(id: string, resultEntryId?: string) {
    return {
      ...call(id),
      input: undefined,
      output: undefined,
      unloaded: true,
      ...(resultEntryId ? { resultEntryId } : {}),
    } satisfies FunctionTriggerMessage
  }

  it('says when earlier messages are loading', () => {
    const html = renderToStaticMarkup(
      <MessageList
        messages={transcript()}
        history={{ hasMore: true, oldestEntryId: 'intro', loadingOlder: true }}
      />,
    )
    expect(html).toContain('data-history-edge="loading"')
    expect(html).toContain('loading earlier messages…')
  })

  it('marks the beginning of the conversation when nothing is above', () => {
    const html = renderToStaticMarkup(
      <MessageList messages={transcript()} history={{ hasMore: false }} />,
    )
    expect(html).toContain('data-history-edge="beginning"')
    expect(html).toContain('beginning of conversation')
  })

  it('offers a retry when the last page failed', () => {
    const html = renderToStaticMarkup(
      <MessageList
        messages={transcript()}
        history={{ hasMore: true, oldestEntryId: 'intro', error: 'timeout' }}
        onLoadOlder={() => {}}
      />,
    )
    expect(html).toContain('data-history-edge="error"')
    expect(html).toContain('earlier messages failed to load')
    expect(html).toContain('>retry</button>')
  })

  /* Nothing loads on its own while the reader sits at the tail, so the edge
     row must offer the way up explicitly: it is the only way for a first
     page too short to scroll, and a visible promise for the rest. */
  it('offers to load earlier messages instead of prefetching them', () => {
    const html = renderToStaticMarkup(
      <MessageList
        messages={transcript()}
        history={{ hasMore: true, oldestEntryId: 'intro' }}
        onLoadOlder={() => {}}
      />,
    )
    expect(html).toContain('data-history-edge="more"')
    expect(html).toContain('>load earlier messages</button>')
  })

  /* The first paint of an open is veiled: the list is laid out (so the tail
     jump and the settle happen for real) but invisible, with the loading
     line in front, until layout goes quiet. What the reader then sees is
     final. */
  it('opens veiled behind a loading line', () => {
    const html = renderToStaticMarkup(
      <MessageList
        messages={transcript()}
        history={{ hasMore: true, oldestEntryId: 'intro' }}
      />,
    )
    expect(html).toContain('data-transcript-opening=""')
    // Opacity, not visibility: status layers set `visibility: visible` on
    // themselves and would show through an inherited `hidden`.
    expect(html).toMatch(/data-message-list=""[^>]*class="[^"]*\bopacity-0\b/)
    expect(html).not.toMatch(
      /data-message-list=""[^>]*class="[^"]*\binvisible\b/,
    )
    expect(html).toContain('data-transcript-loading=""')
    expect(html).toContain('Loading conversation…')
    // The message content is still in the document behind the veil.
    expect(html).toContain('data-message-row=')
  })

  /* An existing session starts with no messages until its durable transcript
     is read. That moment is a load, not a new session: the welcome hero must
     wait for hydration, or it pops in front of a conversation that exists. */
  it('does not show the welcome hero before an existing transcript hydrates', () => {
    const loading = renderToStaticMarkup(
      <MessageList messages={[]} transcriptHydrated={false} />,
    )
    expect(loading).not.toContain('What should we build in')
    expect(loading).toContain('data-transcript-opening=""')
    expect(loading).toContain('data-transcript-loading=""')

    const hydrated = renderToStaticMarkup(
      <MessageList messages={[]} transcriptHydrated />,
    )
    expect(hydrated).toContain('What should we build in')
    expect(hydrated).not.toContain('data-transcript-loading=""')
  })

  it('renders no edge row without history or without messages', () => {
    expect(
      renderToStaticMarkup(<MessageList messages={transcript()} />),
    ).not.toContain('data-history-edge')
    expect(
      renderToStaticMarkup(
        <MessageList messages={[]} history={{ hasMore: false }} isThinking />,
      ),
    ).not.toContain('data-history-edge')
  })

  it('renders a placeholder call as a skeleton card', () => {
    const html = renderToStaticMarkup(
      <MessageList messages={[unloadedCall('p1', 'e_r1')]} defaultOpenCalls />,
    )
    expect(html).toContain('data-unloaded=""')
    expect(html).toContain('data-function-trigger-skeleton=""')
    expect(html).toContain('call p1')
    // No request pane: there are no arguments to show yet.
    expect(html).not.toContain('>request<')
  })

  it('counts placeholders in the group disclosure', () => {
    const html = renderToStaticMarkup(
      <MessageList
        messages={[
          assistant('intro', 'Working through it.', 'function_call'),
          unloadedCall('p1', 'e_r1'),
          unloadedCall('p2', 'e_r2'),
          call('c3'),
        ]}
      />,
    )
    expect(html).toContain('Agent triggered 3 functions')
    expect(html).toContain('show all')
    // Collapsed: only the latest call is mounted; the placeholders wait for
    // "show all" and are never fetched just to be hidden.
    expect(html.match(/data-message-role="function-call"/g)).toHaveLength(1)
    expect(html).not.toContain('data-unloaded=""')
  })

  it('names the entries a placeholder row needs', () => {
    expect(
      unloadedEntryIds([
        unloadedCall('e_a1:2', 'e_r1'),
        unloadedCall('e_a1:3'),
        unloadedCall('e_r9'),
        call('loaded'),
      ]),
    ).toEqual(['e_a1', 'e_r1', 'e_r9'])
  })
})
