import { describe, expect, it } from 'vitest'
import { buildTurnMetadata } from './real'

/**
 * C1 regression: working_dir must ride options.metadata so the harness sees it
 * on the turn record (session metadata is invisible at the dispatch choke
 * point). If this drops, two chats silently collide again — failure mode #3.
 */
describe('buildTurnMetadata — working_dir forwarding', () => {
  it('carries working_dir when a directory is set', () => {
    expect(buildTurnMetadata('s-1', 'm-1', '/proj/A')).toEqual({
      session_id: 's-1',
      message_id: 'm-1',
      working_dir: '/proj/A',
    })
  })

  it('omits working_dir when null, undefined, or empty', () => {
    for (const wd of [null, undefined, ''] as const) {
      expect(buildTurnMetadata('s-1', 'm-1', wd)).toEqual({
        session_id: 's-1',
        message_id: 'm-1',
      })
    }
  })
})
