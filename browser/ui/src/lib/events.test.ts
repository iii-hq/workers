import { describe, expect, it } from 'vitest'
import { BROWSER_ON_FRAME_EVENT_FUNCTION_ID } from './browser'
import { browserEventSource } from './events'

// The console host hands an observer without `capture` the observed event
// itself, and never forwards the removed `deliver` to the engine; the
// sources ask for neither.
describe('browserEventSource', () => {
  it('observes every tab without deliver', () => {
    expect(browserEventSource('browser::on-session-updated', null)).toEqual({
      function_id: 'browser::on-session-updated',
    })
  })

  it('narrows the frame function to a tab through its args', () => {
    expect(browserEventSource(BROWSER_ON_FRAME_EVENT_FUNCTION_ID, 's1')).toEqual({
      function_id: BROWSER_ON_FRAME_EVENT_FUNCTION_ID,
      args: { session_id: 's1' },
    })
  })

  it('narrows every other function to a tab through a filter', () => {
    expect(browserEventSource('browser::on-console-event', 's1')).toEqual({
      function_id: 'browser::on-console-event',
      filters: { 'input.session_id': { $eq: 's1' } },
    })
  })
})
