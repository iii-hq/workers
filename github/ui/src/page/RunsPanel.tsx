import { Badge, type Host } from '@iii-dev/console-ui'
import { useCallback } from 'react'
import { type GithubRun, listRuns, timeAgoIso } from './github-data'
import { type IconProps, Workflow } from './icons'
import { PanelShell } from './PanelShell'
import { useGithubRead } from './useGithubRead'

const RunIcon = (p: IconProps) => <Workflow size={28} {...p} />

type BadgeVariant = 'default' | 'warn' | 'alert' | 'accent'

function runBadge(run: GithubRun): { label: string; variant: BadgeVariant } {
  const status = (run.status ?? '').toLowerCase()
  if (status && status !== 'completed') {
    return { label: status.replace(/_/g, ' '), variant: 'warn' }
  }
  const conclusion = (run.conclusion ?? '').toLowerCase()
  if (conclusion === 'success') return { label: 'success', variant: 'accent' }
  if (
    conclusion === 'failure' ||
    conclusion === 'timed_out' ||
    conclusion === 'startup_failure'
  ) {
    return { label: conclusion.replace(/_/g, ' '), variant: 'alert' }
  }
  return { label: conclusion || 'completed', variant: 'default' }
}

interface RunsPanelProps {
  host: Host
  repo: string
  enabled: boolean
}

/** GitHub Actions runs, newest first (worker default order from gh). */
export function RunsPanel({ host, repo, enabled }: RunsPanelProps) {
  const fetcher = useCallback(() => listRuns(host, repo), [host, repo])
  const { data, loading, error } = useGithubRead(enabled, fetcher)
  const runs = data ?? []

  return (
    <PanelShell
      loading={loading}
      error={error}
      empty={runs.length === 0}
      emptyIcon={RunIcon}
      emptyTitle="no workflow runs"
      emptyDescription={`no recent actions runs in ${repo}`}
    >
      <ul className="gh-list">
        {runs.map((run) => {
          const badge = runBadge(run)
          const meta = [
            run.workflowName,
            run.headBranch ?? undefined,
            timeAgoIso(run.updatedAt ?? run.createdAt) || undefined,
          ]
            .filter(Boolean)
            .join(' · ')
          const title = run.displayTitle ?? run.name ?? String(run.databaseId)
          return (
            <li key={run.databaseId} className="gh-row">
              {run.url ? (
                <a
                  href={run.url}
                  target="_blank"
                  rel="noreferrer"
                  className="gh-row-title"
                >
                  {title}
                </a>
              ) : (
                <span className="gh-row-title">{title}</span>
              )}
              <span className="gh-row-meta gh-trunc-72">{meta}</span>
              <Badge variant={badge.variant}>{badge.label}</Badge>
            </li>
          )
        })}
      </ul>
    </PanelShell>
  )
}
