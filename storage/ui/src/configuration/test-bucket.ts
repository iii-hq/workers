/**
 * "Test bucket" end to end: which request to send, and what its answer says.
 *
 * A draft identical to the stored entry is tested by name alone, so the
 * worker tests what it runs — `${VAR}` templates expanded and legacy literals
 * included, neither of which a request may carry. Any other draft goes as a
 * `bucket` (with its literals in `credentials`). A worker without
 * `storage::getBucket` or without the bucket stored just gets the draft.
 */

import type { Host } from '@iii-dev/console-ui'
import { errorText, isMissingFunction } from '../lib/errors'
import {
  hasTemplates,
  type JsonObject,
  parseTestOutcome,
  sameEntry,
  testBucketPayload,
  testOutcomeHint,
  testOutcomeText,
  underlyingBucket,
} from './bucket'
import type { TestState } from './TestBucketRow'

const TEST_TIMEOUT_MS = 10_000

export async function testRequest(host: Host, name: string, bucket: JsonObject): Promise<Record<string, unknown>> {
  try {
    const stored = await host.iii.trigger<{ bucket?: JsonObject } | null>(
      'storage::getBucket',
      { name },
      { timeoutMs: 10_000 },
    )
    if (stored?.bucket && sameEntry(stored.bucket, bucket)) return { name, timeout_ms: TEST_TIMEOUT_MS }
  } catch {
    // Not stored yet, or a worker that predates getBucket: test the draft.
  }
  return testBucketPayload(name, bucket, TEST_TIMEOUT_MS)
}

/** Run the test for a draft and word its result. */
export async function runBucketTest(host: Host, name: string, bucket: JsonObject): Promise<TestState> {
  try {
    const request = await testRequest(host, name, bucket)
    const raw = await host.iii.trigger<unknown>('storage::testBucket', request, { timeoutMs: 40_000 })
    const outcome = parseTestOutcome(raw)
    if (!outcome) return { status: 'done', ok: false, text: 'Unexpected answer from storage::testBucket' }
    const draftTemplates = request.bucket !== undefined && hasTemplates(bucket)
    return {
      status: 'done',
      ok: outcome.ok,
      text: testOutcomeText(outcome),
      hint: testOutcomeHint(outcome, underlyingBucket(name, bucket), draftTemplates),
    }
  } catch (cause) {
    return isMissingFunction(cause) ? { status: 'unsupported' } : { status: 'done', ok: false, text: errorText(cause) }
  }
}
