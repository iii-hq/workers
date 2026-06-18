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
      deny: ['approval::*', 'configuration::*', 'shell::run', 'state::set'],
      expose: 'agent_trigger',
    })
  })
})
