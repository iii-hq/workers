import { describe, expect, it } from 'vitest'
import type { ValidationLink } from '../../../types'
import { classifyAttachFailure, failedField, idState, lookupKey, primaryAction } from './validation-lookup'

const link = (comparable: boolean) => ({ comparability: { comparable, checks: [] } }) as unknown as ValidationLink

describe('idState', () => {
  it('needs both ids, different, ignoring whitespace', () => {
    expect(idState('', 'exe_2')).toBe('incomplete')
    expect(idState('exe_1', '  ')).toBe('incomplete')
    expect(idState('exe_1', ' exe_1 ')).toBe('same')
    expect(idState('exe_1', 'exe_2')).toBe('ready')
  })

  it('keys a lookup on the trimmed pair', () => {
    expect(lookupKey(' a ', 'b')).toBe(lookupKey('a', ' b '))
    expect(lookupKey('a', 'b')).not.toBe(lookupKey('b', 'a'))
  })
})

describe('classifyAttachFailure', () => {
  it('reads an execution the service could not find', () => {
    const failure = classifyAttachFailure(
      'dependency error: E2E execution exe_9 could not be read: execution not found',
    )
    expect(failure).toMatchObject({ kind: 'not_found', ids: ['exe_9'] })
  })

  it('reads the backend wording of a missing execution as not found, not as a service that is down', () => {
    // `call()` always prefixes the function id, so the reason names e2e::dashboard::execution-get.
    const failure = classifyAttachFailure(
      'dependency error: E2E execution exe_9 could not be read: dependency error: e2e::dashboard::execution-get failed: remote error (NOT_FOUND): execution exe_9 not found',
    )
    expect(failure).toMatchObject({ kind: 'not_found', ids: ['exe_9'] })
  })

  it('reads a service that is not there as unavailable, never as a missing id', () => {
    for (const reason of [
      'function not found: e2e::dashboard::execution-get',
      'dependency error: e2e::dashboard::execution-get failed: remote error (function_not_found): not registered',
      'dependency error: e2e::dashboard::execution-get failed: invocation timed out',
      'dependency error: e2e::dashboard::execution-get exceeded its 10000 ms timeout',
      'dependency error: e2e::dashboard::execution-get failed: iii is not connected',
      'bus connection refused',
    ]) {
      const failure = classifyAttachFailure(`dependency error: E2E execution exe_1 could not be read: ${reason}`)
      expect(failure.kind, reason).toBe('unavailable')
    }
    expect(classifyAttachFailure('the E2E worker is unavailable').kind).toBe('unavailable')
  })

  it('keeps other rejections as plain failures with their message', () => {
    const failure = classifyAttachFailure('evaluation conflict: attach E2E results after the analysis finishes')
    expect(failure).toEqual({
      kind: 'failed',
      message: 'evaluation conflict: attach E2E results after the analysis finishes',
    })
  })

  it('reads the backend codes first and puts a not-found on the side they name', () => {
    const baseline = classifyAttachFailure('dependency error: e2e_execution_not_found(baseline): no such execution')
    expect(baseline).toMatchObject({ kind: 'not_found', fields: ['baseline'] })
    const candidate = classifyAttachFailure('e2e_execution_not_found(candidate)')
    expect(candidate).toMatchObject({ kind: 'not_found', fields: ['candidate'] })
    // The side comes from the code, whatever ids are typed or named in the text.
    const ids = { baseline: 'exe_1', candidate: 'exe_2' }
    expect(failedField(baseline, 'baseline', ids)).toBe(true)
    expect(failedField(baseline, 'candidate', ids)).toBe(false)
    expect(failedField(candidate, 'candidate', ids)).toBe(true)
    expect(failedField(candidate, 'baseline', ids)).toBe(false)
    const both = classifyAttachFailure(
      'e2e_execution_not_found(baseline); e2e_execution_not_found(candidate); e2e_execution_not_found(baseline)',
    )
    expect(both).toMatchObject({ kind: 'not_found', fields: ['baseline', 'candidate'] })
  })

  it('reads e2e_unavailable as a service that is down, ahead of any wording', () => {
    expect(classifyAttachFailure('dependency error: e2e_unavailable: no answer in 10000 ms').kind).toBe('unavailable')
    // A reason that would read as a missing execution by wording alone.
    expect(
      classifyAttachFailure('E2E execution exe_9 could not be read: e2e_unavailable: execution not found').kind,
    ).toBe('unavailable')
  })

  it('reads a code even when the wording says the opposite', () => {
    const failure = classifyAttachFailure('e2e_execution_not_found(candidate): the E2E worker is unavailable')
    expect(failure).toMatchObject({ kind: 'not_found', fields: ['candidate'] })
  })

  it('points a not-found failure at the field that holds the id', () => {
    const failure = classifyAttachFailure('E2E execution exe_9 could not be read: execution not found')
    const ids = { baseline: 'exe_1', candidate: ' exe_9 ' }
    expect(failedField(failure, 'candidate', ids)).toBe(true)
    expect(failedField(failure, 'baseline', ids)).toBe(false)
    expect(failedField(undefined, 'candidate', ids)).toBe(false)
  })
})

describe('primaryAction', () => {
  it('enables Attach only for found, different, comparable runs', () => {
    expect(primaryAction({ phase: 'found', link: link(true) }, 'ready')).toEqual({ kind: 'attach', enabled: true })
    expect(primaryAction({ phase: 'idle' }, 'ready')).toEqual({ kind: 'attach', enabled: false })
    expect(primaryAction({ phase: 'checking' }, 'ready')).toEqual({ kind: 'attach', enabled: false })
    expect(primaryAction({ phase: 'found', link: link(true) }, 'same')).toEqual({ kind: 'attach', enabled: false })
  })

  it('turns Attach into Attach anyway when the runs are not comparable', () => {
    expect(primaryAction({ phase: 'found', link: link(false) }, 'ready')).toEqual({
      kind: 'attach_anyway',
      enabled: true,
    })
  })

  it('offers Try again when the service or the call failed, never when an id is missing', () => {
    const down = classifyAttachFailure('the E2E worker is unavailable')
    const missing = classifyAttachFailure('E2E execution exe_9 could not be read: execution not found')
    expect(primaryAction({ phase: 'failed', failure: down }, 'ready')).toEqual({ kind: 'retry', enabled: true })
    expect(primaryAction({ phase: 'failed', failure: missing }, 'ready')).toEqual({ kind: 'attach', enabled: false })
    const coded = classifyAttachFailure('e2e_execution_not_found(baseline)')
    expect(primaryAction({ phase: 'failed', failure: coded }, 'ready')).toEqual({ kind: 'attach', enabled: false })
    const codedDown = classifyAttachFailure('e2e_unavailable')
    expect(primaryAction({ phase: 'failed', failure: codedDown }, 'ready')).toEqual({ kind: 'retry', enabled: true })
  })

  it('offers Try again while the list of runs is down, whatever the ids say', () => {
    expect(primaryAction({ phase: 'idle' }, 'incomplete', true)).toEqual({ kind: 'retry', enabled: true })
    expect(primaryAction({ phase: 'found', link: link(true) }, 'ready', true)).toEqual({ kind: 'retry', enabled: true })
  })
})
