/**
 * The hook is a thin React shell. All of its decision-making lives
 * in `selectAutoAcceptCandidates` + `nextApprovalsToAutoResolve` +
 * `isAutoAcceptable`, each with dedicated test suites
 * (`auto-accept.test.ts`, `auto-accept-policy.test.ts`).
 *
 * @testing-library/react isn't installed here, so we don't exercise
 * the hook against a real React renderer. The smoke assertions below
 * just pin the export shape; the typechecker catches drift in the
 * imports (if the hook stops calling the policy-aware selector,
 * compilation fails because the selection shape changes).
 */
import { describe, expect, it } from 'vitest'
import { useAutoAcceptApprovals } from './use-auto-accept-approvals'

describe('useAutoAcceptApprovals (wiring guard)', () => {
  it('exports a function', () => {
    expect(typeof useAutoAcceptApprovals).toBe('function')
  })
})
