import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  __resetWorktreeClaimsForTests,
  consoleClaimFor,
  forgetConsoleClaim,
  recordConsoleClaim,
  releaseConsoleClaimIfAny,
  shouldReleaseClaim,
} from './worktree-claims'

afterEach(() => {
  __resetWorktreeClaimsForTests()
})

const claim = { worktreeId: 'wt_1f2e3d4c', path: '/wts/app/wt_1f2e3d4c' }

describe('shouldReleaseClaim', () => {
  it('never releases without a recorded claim', () => {
    expect(shouldReleaseClaim(null, null)).toBe(false)
    expect(shouldReleaseClaim(null, '/anywhere')).toBe(false)
  })

  it('releases on close (no keepPath) and on moving away', () => {
    expect(shouldReleaseClaim(claim, null)).toBe(true)
    expect(shouldReleaseClaim(claim, '/somewhere/else')).toBe(true)
  })

  it('keeps the claim while the conversation still points at it', () => {
    expect(shouldReleaseClaim(claim, claim.path)).toBe(false)
  })
})

describe('releaseConsoleClaimIfAny', () => {
  it('releases only claims made by this console flow', async () => {
    const release = vi.fn().mockResolvedValue(undefined)
    // No claim recorded for this session: nothing fires.
    await releaseConsoleClaimIfAny('console-a', { release })
    expect(release).not.toHaveBeenCalled()

    recordConsoleClaim('console-a', claim)
    await releaseConsoleClaimIfAny('console-b', { release })
    expect(release).not.toHaveBeenCalled()

    await releaseConsoleClaimIfAny('console-a', { release })
    expect(release).toHaveBeenCalledExactlyOnceWith(
      claim.worktreeId,
      'console-a',
    )
    expect(consoleClaimFor('console-a')).toBeNull()
  })

  it('keeps the claim when the working dir still points at the worktree', async () => {
    const release = vi.fn().mockResolvedValue(undefined)
    recordConsoleClaim('console-a', claim)
    await releaseConsoleClaimIfAny('console-a', {
      keepPath: claim.path,
      release,
    })
    expect(release).not.toHaveBeenCalled()
    expect(consoleClaimFor('console-a')).toEqual(claim)
  })

  it('releases when the working dir moves to another path', async () => {
    const release = vi.fn().mockResolvedValue(undefined)
    recordConsoleClaim('console-a', claim)
    await releaseConsoleClaimIfAny('console-a', {
      keepPath: '/home/dev/other',
      release,
    })
    expect(release).toHaveBeenCalledExactlyOnceWith(
      claim.worktreeId,
      'console-a',
    )
    expect(consoleClaimFor('console-a')).toBeNull()
  })

  it('swallows release failures and still forgets the claim', async () => {
    const release = vi.fn().mockRejectedValue(new Error('W211: not owner'))
    recordConsoleClaim('console-a', claim)
    await expect(
      releaseConsoleClaimIfAny('console-a', { release }),
    ).resolves.toBeUndefined()
    expect(consoleClaimFor('console-a')).toBeNull()
  })

  it('forgetConsoleClaim drops the record without releasing', async () => {
    const release = vi.fn().mockResolvedValue(undefined)
    recordConsoleClaim('console-a', claim)
    forgetConsoleClaim('console-a')
    await releaseConsoleClaimIfAny('console-a', { release })
    expect(release).not.toHaveBeenCalled()
  })
})
