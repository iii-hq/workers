import { describe, expect, it } from 'vitest'

import { parseDiscoveryPass, passHeadline } from './pass'

const base = { allowed_functions: 3, functions_generation: 5 }

describe('parseDiscoveryPass', () => {
  it('parses a hint row and forbids a reason on it', () => {
    const pass = parseDiscoveryPass(1, { ...base, outcome: 'hint_injected' })
    expect(pass).toMatchObject({ outcome: 'hint_injected', allowedFunctions: 3 })
    expect(
      parseDiscoveryPass(1, {
        ...base,
        outcome: 'hint_injected',
        reason: 'task_guided',
      }),
    ).toBeNull()
  })

  it('parses every skip reason and rejects unknown ones', () => {
    for (const reason of [
      'search_unavailable',
      'already_searched',
      'narrow_surface',
      'already_operating',
      'task_guided',
      'hint_already_sent',
    ]) {
      expect(
        parseDiscoveryPass(1, { ...base, outcome: 'skipped', reason }),
      ).toMatchObject({ outcome: 'skipped', reason })
    }
    expect(
      parseDiscoveryPass(1, { ...base, outcome: 'skipped', reason: 'nope' }),
    ).toBeNull()
    expect(parseDiscoveryPass(1, { ...base, outcome: 'skipped' })).toBeNull()
  })

  it('rejects other versions and malformed counts', () => {
    expect(parseDiscoveryPass(2, { ...base, outcome: 'hint_injected' })).toBeNull()
    expect(
      parseDiscoveryPass(1, { outcome: 'hint_injected', allowed_functions: -1, functions_generation: 5 }),
    ).toBeNull()
  })
})

describe('passHeadline', () => {
  it('renders the hint line', () => {
    const pass = parseDiscoveryPass(1, { ...base, outcome: 'hint_injected' })!
    expect(passHeadline(pass)).toEqual({
      text: 'discovery injected the search hint',
      detail: 'call discovery::search_functions once',
    })
  })

  it('renders a detail per skip reason', () => {
    const pass = parseDiscoveryPass(1, {
      ...base,
      outcome: 'skipped',
      reason: 'task_guided',
    })!
    expect(passHeadline(pass)).toEqual({
      text: 'discovery injected nothing',
      detail: 'task already names its functions',
    })
  })
})
