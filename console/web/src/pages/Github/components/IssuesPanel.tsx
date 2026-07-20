import { CircleDot } from 'lucide-react'
import { useCallback, useState } from 'react'
import { Badge } from '@/components/ui/Badge'
import { ModeToggle } from '@/components/ui/ModeToggle'
import {
  type GithubIssue,
  ISSUE_STATE_FILTERS,
  type IssueStateFilter,
  listIssues,
  timeAgoIso,
} from '@/lib/github'
import { useGithubQuery } from '../hooks/useGithubQuery'
import { PanelShell } from './PanelShell'

const STATE_OPTIONS: { value: IssueStateFilter; label: string }[] =
  ISSUE_STATE_FILTERS.map((value) => ({ value, label: value }))

function issueBadge(issue: GithubIssue): {
  label: string
  variant: 'accent' | 'default'
} {
  return issue.state.toLowerCase() === 'open'
    ? { label: 'open', variant: 'accent' }
    : { label: 'closed', variant: 'default' }
}

interface IssuesPanelProps {
  repo: string
  enabled: boolean
}

export function IssuesPanel({ repo, enabled }: IssuesPanelProps) {
  const [state, setState] = useState<IssueStateFilter>('open')
  const fetcher = useCallback(() => listIssues(repo, state), [repo, state])
  const { data, loading, error } = useGithubQuery(enabled, fetcher)
  const issues = data ?? []

  return (
    <div className="flex flex-col gap-3">
      <div>
        <ModeToggle<IssueStateFilter>
          value={state}
          onChange={setState}
          options={STATE_OPTIONS}
        />
      </div>
      <PanelShell
        loading={loading}
        error={error}
        empty={issues.length === 0}
        emptyIcon={CircleDot}
        emptyTitle="no issues"
        emptyDescription={`no ${state} issues in ${repo}`}
      >
        <ul className="flex flex-col">
          {issues.map((issue) => {
            const badge = issueBadge(issue)
            return (
              <li
                key={issue.number}
                className="flex items-center gap-3 py-2 border-b border-rule last:border-b-0"
              >
                <span className="font-mono text-[11px] text-ink-faint w-14 shrink-0">
                  #{issue.number}
                </span>
                <a
                  href={issue.url}
                  target="_blank"
                  rel="noreferrer"
                  className="flex-1 min-w-0 truncate font-mono text-[13px] text-ink hover:text-accent transition-colors"
                >
                  {issue.title}
                </a>
                <span className="hidden sm:flex items-center gap-2 font-mono text-[11px] lowercase text-ink-faint shrink-0">
                  {issue.labels?.length ? (
                    <span className="truncate max-w-40">
                      {issue.labels.map((l) => l.name).join(', ')}
                    </span>
                  ) : null}
                  <span className="truncate max-w-44">
                    {issue.author?.login ?? ''}
                    {issue.updatedAt ? ` · ${timeAgoIso(issue.updatedAt)}` : ''}
                  </span>
                </span>
                <Badge variant={badge.variant}>{badge.label}</Badge>
              </li>
            )
          })}
        </ul>
      </PanelShell>
    </div>
  )
}
