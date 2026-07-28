import { Badge, type Host } from '@iii-dev/console-ui'
import { useCallback, useState } from 'react'
import {
  type GithubIssue,
  ISSUE_STATE_FILTERS,
  type IssueStateFilter,
  listIssues,
  timeAgoIso,
} from './github-data'
import { CircleDot, type IconProps } from './icons'
import { ModeToggle } from './ModeToggle'
import { PanelShell } from './PanelShell'
import { useGithubRead } from './useGithubRead'

const STATE_OPTIONS: { value: IssueStateFilter; label: string }[] =
  ISSUE_STATE_FILTERS.map((value) => ({ value, label: value }))

const IssueIcon = (p: IconProps) => <CircleDot size={28} {...p} />

function issueBadge(issue: GithubIssue): {
  label: string
  variant: 'accent' | 'default'
} {
  return issue.state.toLowerCase() === 'open'
    ? { label: 'open', variant: 'accent' }
    : { label: 'closed', variant: 'default' }
}

interface IssuesPanelProps {
  host: Host
  repo: string
  enabled: boolean
}

export function IssuesPanel({ host, repo, enabled }: IssuesPanelProps) {
  const [state, setState] = useState<IssueStateFilter>('open')
  const fetcher = useCallback(
    () => listIssues(host, repo, state),
    [host, repo, state],
  )
  const { data, loading, error } = useGithubRead(enabled, fetcher)
  const issues = data ?? []

  return (
    <div className="gh-panel">
      <div>
        <ModeToggle<IssueStateFilter>
          value={state}
          onChange={setState}
          options={STATE_OPTIONS}
          aria-label="issue state"
        />
      </div>
      <PanelShell
        loading={loading}
        error={error}
        empty={issues.length === 0}
        emptyIcon={IssueIcon}
        emptyTitle="no issues"
        emptyDescription={`no ${state} issues in ${repo}`}
      >
        <ul className="gh-list">
          {issues.map((issue) => {
            const badge = issueBadge(issue)
            return (
              <li key={issue.number} className="gh-row">
                <span className="gh-num">#{issue.number}</span>
                <a
                  href={issue.url}
                  target="_blank"
                  rel="noreferrer"
                  className="gh-row-title"
                >
                  {issue.title}
                </a>
                <span className="gh-row-meta gh-row-meta-group">
                  {issue.labels?.length ? (
                    <span className="gh-labels">
                      {issue.labels.map((l) => l.name).join(', ')}
                    </span>
                  ) : null}
                  <span className="gh-trunc-44">
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
