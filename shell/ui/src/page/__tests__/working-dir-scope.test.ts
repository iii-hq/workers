import { describe, expect, it } from 'vitest'

import {
  workingDirectoryScopeMessage,
  workingDirectoryScopeMismatch,
} from '../working-dir-scope'

describe('Shell and chat working-directory scope', () => {
  it('detects an actionable root outside the paired chat scope', () => {
    expect(
      workingDirectoryScopeMismatch(
        '/private/tmp/project',
        '/repo/harness',
        'session-1',
        true,
      ),
    ).toBe(true)
  })

  it('does not warn without a mismatch, session, or supported host action', () => {
    expect(
      workingDirectoryScopeMismatch('/repo', '/repo', 'session-1', true),
    ).toBe(false)
    expect(workingDirectoryScopeMismatch('/repo', '/other', null, true)).toBe(
      false,
    )
    expect(
      workingDirectoryScopeMismatch('/repo', '/other', 'session-1', false),
    ).toBe(false)
  })

  it('states both scopes clearly', () => {
    expect(workingDirectoryScopeMessage('/new', '/old')).toBe(
      'Browsing /new; chat still works in /old.',
    )
    expect(workingDirectoryScopeMessage('/new', null)).toBe(
      'Browsing /new; chat has no working directory.',
    )
  })
})
