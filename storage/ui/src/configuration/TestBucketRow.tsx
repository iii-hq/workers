/**
 * "Test bucket": builds a throwaway backend from the draft entry in the
 * worker (`storage::testBucket`), with fresh secret resolution, and lists
 * one object at the root. The result says which stage failed — the
 * configuration, the secrets, the endpoint, the credentials, the bucket or
 * the listing — and, for a bucket that doesn't exist yet, what to do. A
 * worker that predates the function gets an explanation instead; the form
 * keeps working without it.
 */

import { Button } from '@iii-dev/console-ui'

export type TestState =
  | { status: 'testing' }
  | { status: 'done'; ok: boolean; text: string; hint?: string | null }
  | { status: 'unsupported' }

export function TestBucketRow({
  state,
  blocker,
  onTest,
}: {
  state: TestState | undefined
  /** Why the test can't run yet ("Fill in the region to test"). */
  blocker: string | null
  onTest: () => void
}) {
  const testing = state?.status === 'testing'
  return (
    <div className="storage-cfg-test">
      <div className="storage-cfg-test-row">
        <Button type="button" size="sm" disabled={testing || blocker !== null} onClick={onTest}>
          {testing ? 'Testing…' : 'Test bucket'}
        </Button>
        {blocker && !testing ? <span className="storage-cfg-secret-status">{blocker}</span> : null}
      </div>
      {state?.status === 'done' ? (
        <div className="storage-cfg-test-result" role="status">
          <span className={state.ok ? 'storage-cfg-test-ok' : 'storage-cfg-test-fail'}>{state.text}</span>
          {state.hint ? <span className="storage-cfg-hint">{state.hint}</span> : null}
        </div>
      ) : null}
      {state?.status === 'unsupported' ? (
        <div className="storage-cfg-notice" role="status">
          This storage worker predates <code>storage::testBucket</code>. Update it (<code>iii worker update storage</code>)
          to test buckets from here; the configuration still saves without it.
        </div>
      ) : null}
    </div>
  )
}
