import { Button } from '@iii-dev/console-ui'
import {
  type ActionKind,
  type ActionRequestResult,
  type ActionStatus,
  isSafeGitHubHttpsUrl,
  type RunStatus,
  type RunSummary,
  type SecurityAction,
  type SecurityRun,
} from './security-scan-data'
import type { SecurityActionsLive } from './useSecurityActions'

function actionStatusCopy(status: ActionStatus): string {
  switch (status) {
    case 'queued':
      return 'queued'
    case 'preparing':
      return 'preparing isolated checkout'
    case 'awaiting_approval':
      return 'waiting for approval'
    case 'completed':
      return 'published'
    case 'failed':
      return 'failed'
    case 'cancelled':
      return 'cancelled'
    default: {
      const exhaustive: never = status
      return exhaustive
    }
  }
}

function ActionStatusLine({ action, request }: { action: SecurityAction | null; request: ActionRequestResult | null }) {
  const current = action ?? request
  if (!current) return null
  const url = action?.result?.url
  const safeUrl = url && isSafeGitHubHttpsUrl(url) ? url : null
  return (
    <p className="security-scan-ui-finding-action-status">
      {current.action === 'issue' ? 'Issue' : 'Fix PR'}: {actionStatusCopy(current.status)}
      {action?.error ? ` — ${action.error.message}` : ''}
      {safeUrl ? (
        <>
          {' '}
          <a href={safeUrl} target="_blank" rel="noreferrer">
            {safeUrl}
          </a>
        </>
      ) : null}
    </p>
  )
}

export function SecurityFindingActions({
  actions,
  runId,
  findingIndex,
  runMode,
  runStatus,
  hasPatch,
  githubConfigured,
}: {
  actions: SecurityActionsLive
  runId: string
  findingIndex: number
  runMode: SecurityRun['mode'] | RunSummary['mode']
  runStatus: RunStatus
  hasPatch: boolean
  githubConfigured: boolean
}) {
  const issue = actions.stateFor(runId, findingIndex, 'issue')
  const fix = actions.stateFor(runId, findingIndex, 'fix_pr')
  let pending: ActionKind | null = null
  if (issue.submitting) {
    pending = 'issue'
  } else if (fix.submitting) {
    pending = 'fix_pr'
  }
  const completed = runStatus === 'completed'
  const canIssue = completed && githubConfigured
  const canFixPr = completed && githubConfigured && runMode === 'suggest' && hasPatch

  const start = (kind: ActionKind) => {
    const confirmMessage =
      kind === 'issue'
        ? 'Create a GitHub issue for this finding? The mutation stays held until you approve it.'
        : 'Create a draft GitHub fix PR for this finding? The mutation stays held until you approve it, and the PR will not merge automatically.'
    if (!window.confirm(confirmMessage)) return
    void actions.request(runId, findingIndex, kind)
  }

  return (
    <div className="security-scan-ui-finding-actions">
      <div className="security-scan-ui-finding-action-row">
        <Button variant="ghost" size="sm" disabled={!canIssue || pending !== null} onClick={() => start('issue')}>
          {pending === 'issue' ? 'starting issue' : 'Create issue'}
        </Button>
        {canFixPr ? (
          <Button variant="ghost" size="sm" disabled={pending !== null} onClick={() => start('fix_pr')}>
            {pending === 'fix_pr' ? 'starting fix PR' : 'Create fix PR'}
          </Button>
        ) : null}
      </div>
      {!githubConfigured && completed ? (
        <p>GitHub actions need an operator-verified github.full_name mapping.</p>
      ) : null}
      <ActionStatusLine action={issue.action} request={issue.request} />
      <ActionStatusLine action={fix.action} request={fix.request} />
      {issue.error || fix.error ? <p role="alert">{issue.error ?? fix.error}</p> : null}
    </div>
  )
}
