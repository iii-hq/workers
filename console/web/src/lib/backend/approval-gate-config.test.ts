import { describe, expect, it } from 'vitest'
import {
  autoAllowSeedFromRules,
  deriveFunctionPolicy,
} from './approval-gate-config'

describe('approval-gate-config', () => {
  it('extracts auto-scoped allow rules as the seed allowlist', () => {
    expect(
      autoAllowSeedFromRules([
        { function: 'state::get', action: 'allow', modes: ['auto'] },
        { function: 'shell::run', action: 'allow' },
        '!approval::*',
      ]),
    ).toEqual(['state::get'])
  })

  it('derives harness deny globs from deployment rules', () => {
    expect(
      deriveFunctionPolicy([
        { function: 'shell::run', action: 'deny' },
        '!state::set',
      ]),
    ).toEqual({
      allow: ['*'],
      deny: [
        'approval::*',
        'configuration::register',
        'configuration::set',
        'shell::run',
        'state::set',
      ],
      expose: 'agent_trigger',
    })
  })

  it('denies configuration mutations without blocking reads', () => {
    const deny = deriveFunctionPolicy([]).deny

    expect(deny).toEqual([
      'approval::*',
      'configuration::register',
      'configuration::set',
    ])
    expect(deny).not.toContain('configuration::*')
    expect(deny).not.toContain('configuration::get')
  })

  it.each([
    ['shorthand', '!configuration::get'],
    ['structured', { function: 'configuration::get', action: 'deny' }],
  ])('preserves an explicit %s deny for configuration::get', (_, rule) => {
    expect(deriveFunctionPolicy([rule]).deny).toContain('configuration::get')
  })
})
