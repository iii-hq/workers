import { Workflow } from 'lucide-react'
import { useCallback } from 'react'
import { Badge } from '@/components/ui/Badge'
import { type GithubRun, listRuns, timeAgoIso } from '@/lib/github'
import { useGithubQuery } from '../hooks/useGithubQuery'
import { PanelShell } from './PanelShell'

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
  repo: string
  enabled: boolean
}

/** GitHub Actions runs, newest first (worker default order from gh). */
export function RunsPanel({ repo, enabled }: RunsPanelProps) {
  const fetcher = useCallback(() => listRuns(repo), [repo])
  const { data, loading, error } = useGithubQuery(enabled, fetcher)
  const runs = data ?? []

  return (
    <PanelShell
      loading={loading}
      error={error}
      empty={runs.length === 0}
      emptyIcon={Workflow}
      emptyTitle="no workflow runs"
      emptyDescription={`no recent actions runs in ${repo}`}
    >
      <ul className="flex flex-col">
        {runs.map((run) => {
          const badge = runBadge(run)
          const meta = [
            run.workflowName,
            run.headBranch ?? undefined,
            timeAgoIso(run.updatedAt ?? run.createdAt) || undefined,
          ]
            .filter(Boolean)
            .join(' · ')
          return (
            <li
              key={run.databaseId}
              className="flex items-center gap-3 py-2 border-b border-rule last:border-b-0"
            >
              {run.url ? (
                <a
                  href={run.url}
                  target="_blank"
                  rel="noreferrer"
                  className="flex-1 min-w-0 truncate font-mono text-[13px] text-ink hover:text-accent transition-colors"
                >
                  {run.displayTitle ?? run.name ?? String(run.databaseId)}
                </a>
              ) : (
                <span className="flex-1 min-w-0 truncate font-mono text-[13px] text-ink">
                  {run.displayTitle ?? run.name ?? String(run.databaseId)}
                </span>
              )}
              <span className="hidden sm:block font-mono text-[11px] lowercase text-ink-faint truncate max-w-72 shrink-0">
                {meta}
              </span>
              <Badge variant={badge.variant}>{badge.label}</Badge>
            </li>
          )
        })}
      </ul>
    </PanelShell>
  )
}
