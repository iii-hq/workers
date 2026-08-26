import { describe, expect, it } from 'vitest'
import { FIRST_PARTY_RENDERERS } from '@/components/function-trigger/renderer-registry'
import type { FunctionTriggerMessage } from '@/types/chat'
import { AgentSessionToolView, agentRunResponse, requestedTask } from './index'

function message(
  functionId: string,
  output: unknown,
  input: unknown = {},
): FunctionTriggerMessage {
  return {
    id: 'm1',
    role: 'function-call',
    functionId,
    input,
    output,
    createdAt: 1,
  } as unknown as FunctionTriggerMessage
}

describe('an agent run is recognised by its response', () => {
  it('reads the session of every shape an agent run answers with', () => {
    // the spawn acknowledgement
    expect(
      agentRunResponse({ child_session_id: 'c1', child_turn_id: 't1' }),
    ).toEqual({
      sessionId: 'c1',
      started: true,
    })
    // a run accepted and left running (`claude::start`)
    expect(agentRunResponse({ session_id: 's1', started: true })).toEqual({
      sessionId: 's1',
      started: true,
    })
    // a run that finished (`run::start_and_wait`)
    expect(
      agentRunResponse({
        session_id: 's1',
        result: 'done',
        usage: { input_tokens: 3 },
      }),
    ).toEqual({ sessionId: 's1', started: false })
  })

  it('leaves every other kind of session alone', () => {
    // A PTY session is a session, and it is not an agent run. Claiming it
    // would put a sub-agent card on a terminal.
    expect(
      agentRunResponse({ session_id: 's1', access_key: 'k', pid: 42 }),
    ).toBeNull()
    expect(agentRunResponse({ ok: true })).toBeNull()
    expect(agentRunResponse(null)).toBeNull()
    expect(agentRunResponse('done')).toBeNull()
  })

  it('takes the task from whichever field the request uses', () => {
    expect(requestedTask({ prompt: 'fix the flaky test' })).toBe(
      'fix the flaky test',
    )
    expect(requestedTask({ task: 'review the diff' })).toBe('review the diff')
    expect(
      requestedTask({
        messages: [
          { role: 'user', content: 'first' },
          { role: 'assistant', content: 'ignored' },
          { role: 'user', content: [{ type: 'text', text: 'last' }] },
        ],
      }),
    ).toBe('last')
    expect(requestedTask({})).toBeNull()
  })
})

describe('the renderer claims a run and nothing else', () => {
  it('renders for any worker that answers with a child session', () => {
    expect(
      AgentSessionToolView.tryRender(
        message('claude::start', { session_id: 's1', started: true }),
      ),
    ).not.toBeNull()
    expect(
      AgentSessionToolView.tryRender(
        message('some-worker::run', { session_id: 's1', result: 'done' }),
      ),
    ).not.toBeNull()
  })

  it('stays out of the way of everything else', () => {
    expect(
      AgentSessionToolView.tryRender(
        message('shell::pty::open', { session_id: 's1', access_key: 'k' }),
      ),
    ).toBeNull()
    expect(
      AgentSessionToolView.tryRender(message('state::set', { ok: true })),
    ).toBeNull()
  })

  it('waits for the answer', () => {
    const running = {
      ...message('claude::start', undefined),
      running: true,
    } as FunctionTriggerMessage
    expect(AgentSessionToolView.tryRender(running)).toBeNull()
  })

  it('is registered last, so every family keeps its own ids', () => {
    const ids = FIRST_PARTY_RENDERERS.map((renderer) => renderer.id)
    expect(ids.at(-1)).toBe('first-party/agent-session')
    const entry = FIRST_PARTY_RENDERERS.at(-1)
    // Matching every id must cost nothing anywhere else. `display` is the one
    // that would: prominence is read off `isMatch` alone, so a matches-
    // everything renderer carrying it would mark every call in a transcript
    // prominent and stop sequences from collapsing.
    expect(entry?.metadata).toBeUndefined()
    expect(entry?.FunctionIdLabel).toBeUndefined()
    expect(entry?.redactRaw).toBeUndefined()
    expect(entry?.tryRenderPreview).toBeUndefined()
    expect(entry?.tryRenderDisplay).toBeUndefined()
  })
})
