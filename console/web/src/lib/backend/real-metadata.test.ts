import { describe, expect, it } from 'vitest'
import { buildTurnMetadata, FALLBACK_FUNCTION_POLICY } from './real'

describe('buildTurnMetadata — fs_scope forwarding', () => {
  it('carries fs_scope.root when a directory is set', () => {
    expect(buildTurnMetadata('s-1', 'm-1', '/proj/A')).toEqual({
      session_id: 's-1',
      message_id: 'm-1',
      fs_scope: { root: '/proj/A' },
    })
  })

  it('omits fs_scope when null, undefined, or empty', () => {
    for (const wd of [null, undefined, ''] as const) {
      expect(buildTurnMetadata('s-1', 'm-1', wd)).toEqual({
        session_id: 's-1',
        message_id: 'm-1',
      })
    }
  })
})

describe('fallback function policy', () => {
  it('does not expose workspace picker functions to the agent', () => {
    expect(FALLBACK_FUNCTION_POLICY.deny).toContain('shell::workspace::*')
  })
})
