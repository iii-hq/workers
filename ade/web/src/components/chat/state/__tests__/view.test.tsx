import { describe, expect, it } from 'vitest'
import { SandboxErrorView } from '@/components/chat/sandbox/ErrorView'
import type { FunctionCallMessage } from '@/types/chat'
import { StateToolView } from '../index'
import { StateView } from '../StateView'

function wrap<T>(details: T) {
  return {
    content: [{ type: 'text', text: JSON.stringify(details) }],
    details,
    terminate: false,
  }
}

function msg(output: unknown): FunctionCallMessage {
  return {
    id: 'state-view-test',
    role: 'function-call',
    functionId: 'state::get',
    input: { scope: 'ops', key: 'gate' },
    output,
    durationMs: 1,
    createdAt: 0,
  }
}

/** Regression: state values are arbitrary JSON, so a *successful* get whose
 *  stored value merely looks like a denial (`{ status: 'denied' }`) must render
 *  as the value view — not the red error view — while a genuine
 *  `function_error` envelope still surfaces as the error view. */
describe('StateToolView.tryRender error discrimination', () => {
  it('renders a denial-shaped stored value as the value view, not an error', () => {
    const node = StateToolView.tryRender(
      msg(wrap({ status: 'denied', reason: 'gate closed' })),
    )
    expect(node).not.toBeNull()
    expect((node as { type: unknown }).type).toBe(StateView)
  })

  it('renders a { denied_by } stored value as the value view, not an error', () => {
    const node = StateToolView.tryRender(msg(wrap({ denied_by: 'user' })))
    expect((node as { type: unknown }).type).toBe(StateView)
  })

  it('still renders a real function_error envelope as the error view', () => {
    const node = StateToolView.tryRender(
      msg({
        error: {
          kind: 'function_error',
          message: 'boom',
          details: { status: 'denied', denied_by: 'gate_unavailable' },
        },
      }),
    )
    expect(node).not.toBeNull()
    expect((node as { type: unknown }).type).toBe(SandboxErrorView)
  })
})
