import { describe, expect, it } from 'vitest'
import {
  missingAfterChanges,
  missingFromStats,
  NO_MISSING,
  pruneMissing,
  withMissing,
  withMissingPaths,
} from '../missing-files'

describe('missing files', () => {
  it('marks and clears one path, keeping the instance when nothing changes', () => {
    const marked = withMissing(NO_MISSING, 'src/a.ts', true)
    expect([...marked]).toEqual(['src/a.ts'])
    expect(withMissing(marked, 'src/a.ts', true)).toBe(marked)
    expect(withMissing(NO_MISSING, 'src/a.ts', false)).toBe(NO_MISSING)
    expect([...withMissing(marked, 'src/a.ts', false)]).toEqual([])
  })

  it('marks a probe answer at once', () => {
    const marked = withMissingPaths(NO_MISSING, ['a', 'b'])
    expect([...marked].sort()).toEqual(['a', 'b'])
    expect(withMissingPaths(marked, ['a'])).toBe(marked)
    expect(withMissingPaths(marked, [])).toBe(marked)
  })

  it('follows the live feed: a delete marks open tabs, a create or modify clears them', () => {
    const open = new Set(['src/a.ts', 'src/b.ts', 'README.md'])
    const afterDelete = missingAfterChanges(NO_MISSING, [{ rel: 'src/a.ts', kind: 'deleted', dir: false }], open)
    expect([...afterDelete]).toEqual(['src/a.ts'])
    // A file that is not open is nobody's concern.
    expect(missingAfterChanges(NO_MISSING, [{ rel: 'src/zzz.ts', kind: 'deleted', dir: false }], open)).toBe(NO_MISSING)
    // A deleted folder takes every open tab under it.
    const afterDirDelete = missingAfterChanges(NO_MISSING, [{ rel: 'src', kind: 'deleted', dir: true }], open)
    expect([...afterDirDelete].sort()).toEqual(['src/a.ts', 'src/b.ts'])
    // Back on disk (an atomic replace reads as delete then create).
    const back = missingAfterChanges(afterDirDelete, [{ rel: 'src/a.ts', kind: 'created', dir: false }], open)
    expect([...back]).toEqual(['src/b.ts'])
    expect(missingAfterChanges(back, [{ rel: 'src/b.ts', kind: 'modified', dir: false }], open)).toEqual(new Set())
    // Nothing relevant: same instance.
    expect(missingAfterChanges(back, [{ rel: 'other', kind: 'created', dir: false }], open)).toBe(back)
  })

  it('forgets closed tabs', () => {
    const marked = withMissingPaths(NO_MISSING, ['a', 'b'])
    expect(pruneMissing(marked, new Set(['a', 'b', 'c']))).toBe(marked)
    expect([...pruneMissing(marked, new Set(['b']))]).toEqual(['b'])
  })

  it('reads a batch stat back into root-relative paths', () => {
    expect(
      missingFromStats(
        [
          { path: '/repo/src/a.ts', success: true },
          { path: '/repo/src/gone.ts', success: false },
          { path: '/elsewhere/x', success: false },
        ],
        '/repo',
      ),
    ).toEqual(['src/gone.ts', '/elsewhere/x'])
    expect(missingFromStats([{ path: '/repo/x', success: false }], '/repo/')).toEqual(['x'])
  })
})
