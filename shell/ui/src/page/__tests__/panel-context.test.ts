import { describe, expect, it } from 'vitest'
import { parseShellPanelContext } from '../panel-context'

describe('parseShellPanelContext', () => {
  it('accepts file and exact-diff events', () => {
    expect(parseShellPanelContext({ type: 'file', path: '/repo/a.ts' })).toEqual(
      { type: 'file', path: '/repo/a.ts' },
    )
    expect(
      parseShellPanelContext({
        type: 'change-diff',
        changeId: 'snapshot-1',
        path: '/repo/a.ts',
        canViewFile: true,
      }),
    ).toEqual({
      type: 'change-diff',
      changeId: 'snapshot-1',
      path: '/repo/a.ts',
      canViewFile: true,
    })
  })

  it('rejects malformed or worker-foreign context', () => {
    expect(parseShellPanelContext(null)).toBeNull()
    expect(parseShellPanelContext({ type: 'file', path: '' })).toBeNull()
    expect(
      parseShellPanelContext({ type: 'change-diff', path: '/repo/a.ts' }),
    ).toBeNull()
    expect(parseShellPanelContext({ type: 'screenshot', path: '/x' })).toBeNull()
  })
})
