import { Badge, type Host } from '@iii-dev/console-ui'
import { useCallback, useState } from 'react'
import {
  type GithubPr,
  type GithubPrCheck,
  listPrChecks,
  listPrs,
  PR_STATE_FILTERS,
  type PrStateFilter,
  timeAgoIso,
} from './github-data'
import { GitPullRequest, type IconProps } from './icons'
import { ModeToggle } from './ModeToggle'
import { PanelShell } from './PanelShell'
import { useGithubRead } from './useGithubRead'

const STATE_OPTIONS: { value: PrStateFilter; label: string }[] =
  PR_STATE_FILTERS.map((value) => ({ value, label: value }))

const PrIcon = (p: IconProps) => <GitPullRequest size={28} {...p} />

type BadgeVariant = 'default' | 'warn' | 'alert' | 'accent'

function prBadge(pr: GithubPr): { label: string; variant: BadgeVariant } {
  const state = pr.state.toLowerCase()
  if (pr.isDraft && state === 'open')
    return { label: 'draft', variant: 'default' }
  if (state === 'open') return { label: 'open', variant: 'accent' }
  if (state === 'merged') return { label: 'merged', variant: 'default' }
  if (state === 'closed') return { label: 'closed', variant: 'alert' }
  return { label: state, variant: 'default' }
}

function checkVariant(bucket: string): BadgeVariant {
  switch (bucket) {
    case 'pass':
      return 'accent'
    case 'fail':
      return 'alert'
    case 'pending':
      return 'warn'
    default:
      return 'default'
  }
}

interface PrsPanelProps {
  host: Host
  repo: string
  enabled: boolean
}

/** Pull requests for the selected repo; a row expands its CI check rollup. */
export function PrsPanel({ host, repo, enabled }: PrsPanelProps) {
  const [state, setState] = useState<PrStateFilter>('open')
  const [expanded, setExpanded] = useState<number | null>(null)
  const fetcher = useCallback(
    () => listPrs(host, repo, state),
    [host, repo, state],
  )
  const { data, loading, error } = useGithubRead(enabled, fetcher)
  const prs = data ?? []

  return (
    <div className="gh-panel">
      <div>
        <ModeToggle<PrStateFilter>
          value={state}
          onChange={setState}
          options={STATE_OPTIONS}
          aria-label="pull request state"
        />
      </div>
      <PanelShell
        loading={loading}
        error={error}
        empty={prs.length === 0}
        emptyIcon={PrIcon}
        emptyTitle="no pull requests"
        emptyDescription={`no ${state} pull requests in ${repo}`}
      >
        <ul className="gh-list">
          {prs.map((pr) => {
            const badge = prBadge(pr)
            const isExpanded = expanded === pr.number
            return (
              <li key={pr.number} className="gh-row-outer">
                <div className="gh-row">
                  <button
                    type="button"
                    onClick={() =>
                      setExpanded((cur) =>
                        cur === pr.number ? null : pr.number,
                      )
                    }
                    aria-expanded={isExpanded}
                    title="toggle checks"
                    className="gh-num gh-num-btn"
                  >
                    #{pr.number}
                  </button>
                  <a
                    href={pr.url}
                    target="_blank"
                    rel="noreferrer"
                    className="gh-row-title"
                  >
                    {pr.title}
                  </a>
                  <span className="gh-row-meta">
                    {pr.author?.login ?? ''}
                    {pr.updatedAt ? ` · ${timeAgoIso(pr.updatedAt)}` : ''}
                  </span>
                  <Badge variant={badge.variant}>{badge.label}</Badge>
                </div>
                {isExpanded ? (
                  <ChecksInline host={host} repo={repo} number={pr.number} />
                ) : null}
              </li>
            )
          })}
        </ul>
      </PanelShell>
    </div>
  )
}

function ChecksInline({
  host,
  repo,
  number,
}: {
  host: Host
  repo: string
  number: number
}) {
  const fetcher = useCallback(
    () => listPrChecks(host, repo, number),
    [host, repo, number],
  )
  const { data, loading, error } = useGithubRead(true, fetcher)
  const checks = data ?? []

  return (
    <div className="gh-checks">
      {error ? (
        <p className="gh-checks-msg alert">{error}</p>
      ) : loading && checks.length === 0 ? (
        <p className="gh-checks-msg ghost">loading checks…</p>
      ) : checks.length === 0 ? (
        <p className="gh-checks-msg">no checks reported</p>
      ) : (
        <ul className="gh-checks-list">
          {checks.map((check) => (
            <li key={checkKey(check)} className="gh-check">
              <Badge variant={checkVariant(check.bucket)}>{check.bucket}</Badge>
              {check.link ? (
                <a
                  href={check.link}
                  target="_blank"
                  rel="noreferrer"
                  className="gh-check-label link"
                >
                  {checkLabel(check)}
                </a>
              ) : (
                <span className="gh-check-label">{checkLabel(check)}</span>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

function checkLabel(check: GithubPrCheck): string {
  return check.workflow ? `${check.workflow} / ${check.name}` : check.name
}

function checkKey(check: GithubPrCheck): string {
  return `${check.workflow ?? ''}/${check.name}/${check.startedAt ?? ''}`
}
