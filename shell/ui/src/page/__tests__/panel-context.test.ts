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

  it('carries a line window when the reference names one', () => {
    expect(parseShellPanelContext({ type: 'file', path: '/repo/a.ts', line: 12, endLine: 40 })).toEqual({
      type: 'file',
      path: '/repo/a.ts',
      line: 12,
      endLine: 40,
    })
    expect(parseShellPanelContext({ type: 'file', path: 'a.ts', line: 7 })).toEqual({ type: 'file', path: 'a.ts', line: 7 })
    // An end before the start, a zero, or a stray end without a start is dropped, not fatal.
    expect(parseShellPanelContext({ type: 'file', path: 'a.ts', line: 9, endLine: 3 })).toEqual({ type: 'file', path: 'a.ts', line: 9 })
    expect(parseShellPanelContext({ type: 'file', path: 'a.ts', line: 0 })).toEqual({ type: 'file', path: 'a.ts' })
    expect(parseShellPanelContext({ type: 'file', path: 'a.ts', endLine: 3 })).toEqual({ type: 'file', path: 'a.ts' })
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
