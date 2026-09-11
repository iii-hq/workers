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
        'shell::run',
        'shell::workspace::*',
        'state::set',
      ],
      expose: 'agent_trigger',
    })
  })

  it('keeps the structural floor in lockstep with the fallback policy', () => {
    const deny = deriveFunctionPolicy([]).deny

    expect(deny).toEqual([
      'approval::*',
      'configuration::register',
      'shell::workspace::*',
    ])
    expect(deny).not.toContain('configuration::*')
    expect(deny).not.toContain('configuration::get')
    expect(deny).not.toContain('configuration::set')
  })

  it.each([
    ['shorthand', '!configuration::get'],
    ['structured', { function: 'configuration::get', action: 'deny' }],
  ])('preserves an explicit %s deny for configuration::get', (_, rule) => {
    expect(deriveFunctionPolicy([rule]).deny).toContain('configuration::get')
  })

  it.each([
    ['shorthand', '!configuration::set'],
    ['structured', { function: 'configuration::set', action: 'deny' }],
  ])('preserves an explicit %s deny for configuration::set', (_, rule) => {
    expect(deriveFunctionPolicy([rule]).deny).toContain('configuration::set')
  })
})
