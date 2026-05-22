/**
 * The hook itself is a thin React shell. All of its decision-making
 * lives in `selectAutoAcceptCandidates` + `nextApprovalsToAutoResolve`
 * + `isAutoAcceptable`. Those have their own dedicated test suites
 * (`auto-accept.test.ts`, `auto-accept-policy.test.ts`). This file
 * documents the contract the hook commits to so the wiring stays
 * honest without pulling in `@testing-library/react`.
 *
 * If you change hook behaviour, please update one of the following:
 *  - the pure helper's tests (preferred — most behaviour is there)
 *  - this file's "guard" assertions (re-export shape, signature
 *    drift) so the hook can't silently lose policy enforcement.
 */
import { describe, expect, it } from 'vitest'
import { useAutoAcceptApprovals } from './use-auto-accept-approvals'

describe('useAutoAcceptApprovals (wiring guard)', () => {
  it('exports a function', () => {
    expect(typeof useAutoAcceptApprovals).toBe('function')
  })

  it('imports the policy-aware selector from auto-accept', async () => {
    // If a refactor accidentally drops the policy filter, this guard
    // catches it: the hook MUST import the policy-aware selector.
    const fs = await import('node:fs/promises')
    const src = await fs.readFile(
      new URL('./use-auto-accept-approvals.ts', import.meta.url),
      'utf8',
    )
    expect(src).toMatch(/selectAutoAcceptCandidates/)
    expect(src).toMatch(/isAutoAcceptable|DEFAULT_POLICY|AutoAcceptPolicy/)
  })
})
