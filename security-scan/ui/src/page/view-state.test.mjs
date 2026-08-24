import assert from 'node:assert/strict'
import test from 'node:test'
import {
  analysisConversationFromSession,
  automaticFocusTarget,
  beginRetry,
  isRepositoryScopeCurrent,
  isStreamLive,
  nextVisibleFindingCount,
  scanHistoryDescription,
  settleRetry,
  shouldFollowAnalysisChat,
  shouldReloadReconciliation,
} from './view-state.js'

test('restores only run-linked security review conversations after reload', () => {
  assert.deepEqual(
    analysisConversationFromSession({
      session_id: 'security-scan-analysis-nonce-attempt-1',
      title: 'Security review',
      metadata: {
        security_scan: true,
        security_scan_run_id: 'sec_123',
      },
    }),
    {
      sessionId: 'security-scan-analysis-nonce-attempt-1',
      runId: 'sec_123',
    },
  )
  assert.equal(
    analysisConversationFromSession({
      session_id: 'security-scan-analysis-legacy-attempt-1',
      title: 'Security review',
      metadata: { security_scan: true },
    }),
    null,
  )
  assert.equal(
    analysisConversationFromSession({
      session_id: 'unrelated',
      title: 'Security review',
      metadata: {
        security_scan: true,
        security_scan_run_id: 'sec_123',
      },
    }),
    null,
  )
})

test('follows the analysis chat only for the scan the user started', () => {
  assert.equal(
    shouldFollowAnalysisChat({
      followRunId: 'run-1',
      startConversationId: 'draft-1',
      currentConversationId: 'draft-1',
    }),
    true,
  )
  assert.equal(
    shouldFollowAnalysisChat({
      followRunId: null,
      startConversationId: 'draft-1',
      currentConversationId: 'draft-1',
    }),
    false,
  )
  assert.equal(
    shouldFollowAnalysisChat({
      followRunId: 'run-1',
      startConversationId: 'draft-1',
      currentConversationId: 'some-other-chat',
    }),
    false,
  )
  assert.equal(
    shouldFollowAnalysisChat({
      followRunId: 'run-1',
      startConversationId: 'draft-1',
      currentConversationId: 'security-scan-analysis-abc',
    }),
    true,
  )
})

test('keeps concurrent retry state isolated by run id', () => {
  let states = beginRetry({}, 'run-a')
  states = beginRetry(states, 'run-b')
  states = settleRetry(states, 'run-a', 'retry failed')

  assert.deepEqual(states['run-a'], { pending: false, error: 'retry failed' })
  assert.deepEqual(states['run-b'], { pending: true, error: null })
})

test('reports live only for a registered binding on a connected host', () => {
  assert.equal(isStreamLive(true, 'connected'), true)
  assert.equal(isStreamLive(true, 'reconnecting'), false)
  assert.equal(isStreamLive(false, 'connected'), false)
})

test('keeps the scan history summary readable in narrow panes', () => {
  assert.equal(scanHistoryDescription(3, false), '3 recent repository reviews')
  assert.equal(scanHistoryDescription(3, true), '3 runs')
  assert.equal(scanHistoryDescription(1, true), '1 run')
})

test('withholds repository-scoped state until the active repository resolves', () => {
  assert.equal(isRepositoryScopeCurrent('iii-hq/iii', null), false)
  assert.equal(isRepositoryScopeCurrent('iii-hq/workers', 'iii-hq/iii'), false)
  assert.equal(
    isRepositoryScopeCurrent('iii-hq/workers', 'iii-hq/workers'),
    true,
  )
  assert.equal(isRepositoryScopeCurrent('', ''), true)
})

test('chooses a stable automatic narrow-pane focus target', () => {
  assert.deepEqual(automaticFocusTarget(true, true, 'run-next'), {
    kind: 'run',
    runId: 'run-next',
  })
  assert.deepEqual(automaticFocusTarget(true, true, null), { kind: 'filter' })
  assert.equal(automaticFocusTarget(false, true, 'run-next'), null)
  assert.equal(automaticFocusTarget(true, false, 'run-next'), null)
})

test('progressively reveals findings without exceeding the report size', () => {
  assert.equal(nextVisibleFindingCount(20, 55, 20), 40)
  assert.equal(nextVisibleFindingCount(40, 55, 20), 55)
})

test('reloads reconciliation only for a selected run after a newer refresh signal', () => {
  assert.equal(shouldReloadReconciliation(4, 5, 'run-a'), true)
  assert.equal(shouldReloadReconciliation(5, 5, 'run-a'), false)
  assert.equal(shouldReloadReconciliation(4, 5, null), false)
})
