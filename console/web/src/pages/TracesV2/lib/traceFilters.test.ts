import { describe, expect, it } from 'vitest'
import { withSessionScope } from './traceFilters'

describe('withSessionScope', () => {
  it('is a no-op without a session', () => {
    const params = { service_name: 'harness' }
    expect(withSessionScope(params, null)).toBe(params)
  })

  it('adds the session attribute AND the all-spans search shape', () => {
    // The identity attrs live on child spans — without search_all_spans a
    // roots-only attribute filter matches nothing (verified live).
    expect(withSessionScope({}, 'console-1')).toEqual({
      attributes: [['iii.session.id', 'console-1']],
      search_all_spans: true,
    })
  })

  it('appends after user attribute filters instead of replacing them', () => {
    const scoped = withSessionScope(
      { attributes: [['iii.tag.kind', 'harness.turn']] },
      'console-1',
    )
    expect(scoped.attributes).toEqual([
      ['iii.tag.kind', 'harness.turn'],
      ['iii.session.id', 'console-1'],
    ])
  })

  it('does not mutate the input params', () => {
    const params = { attributes: [['a', 'b']] as [string, string][] }
    withSessionScope(params, 'console-1')
    expect(params.attributes).toEqual([['a', 'b']])
  })
})
