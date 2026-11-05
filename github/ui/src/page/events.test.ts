import { describe, expect, it } from 'vitest'
import { type CalledEvent, calledEventOf, ON_CALLED_FN, ON_CALLED_SOURCE } from './events'

const event: CalledEvent = {
  function_id: 'github::pr::list',
  args_summary: 'pr list --repo=o/r',
  repo: 'o/r',
  ok: true,
  duration_ms: 12,
  result_summary: '2 pull requests',
  kind: 'list',
  result_preview: [{ number: 1 }, { number: 2 }],
  timestamp: '2026-01-01T00:00:00Z',
}

/** What a `capture: ['output']` observer target receives. */
function envelope(output?: unknown) {
  return {
    observer_id: 'obs_1',
    activation_id: 'act_1',
    function_id: ON_CALLED_FN,
    invocation_id: 'inv_1',
    status: 'success',
    started_at_ms: 1,
    duration_ms: 0.5,
    ...(output === undefined ? {} : { output }),
  }
}

describe('github::on-called observer', () => {
  it('captures the output and never sets the removed deliver', () => {
    expect(ON_CALLED_SOURCE).toEqual({ function_id: ON_CALLED_FN, capture: ['output'] })
    expect(ON_CALLED_SOURCE).not.toHaveProperty('deliver')
  })

  it('reads the event from the observed envelope', () => {
    expect(calledEventOf(envelope(event))).toEqual(event)
  })

  it('skips an envelope without an output', () => {
    expect(calledEventOf(envelope())).toBeUndefined()
  })
})
