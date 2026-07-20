import { GitPullRequest } from 'lucide-react'
import { useCallback, useState } from 'react'
import { Badge } from '@/components/ui/Badge'
import { ModeToggle } from '@/components/ui/ModeToggle'
import {
  type GithubPr,
  type GithubPrCheck,
  listPrChecks,
  listPrs,
  PR_STATE_FILTERS,
  type PrStateFilter,
  timeAgoIso,
} from '@/lib/github'
import { useGithubQuery } from '../hooks/useGithubQuery'
import { PanelShell } from './PanelShell'

const STATE_OPTIONS: { value: PrStateFilter; label: string }[] =
  PR_STATE_FILTERS.map((value) => ({ value, label: value }))

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
  repo: string
  enabled: boolean
}

/** Pull requests for the selected repo; a row expands its CI check rollup. */
export function PrsPanel({ repo, enabled }: PrsPanelProps) {
  const [state, setState] = useState<PrStateFilter>('open')
  const [expanded, setExpanded] = useState<number | null>(null)
  const fetcher = useCallback(() => listPrs(repo, state), [repo, state])
  const { data, loading, error } = useGithubQuery(enabled, fetcher)
  const prs = data ?? []

  return (
    <div className="flex flex-col gap-3">
      <div>
        <ModeToggle<PrStateFilter>
          value={state}
          onChange={setState}
          options={STATE_OPTIONS}
        />
      </div>
      <PanelShell
        loading={loading}
        error={error}
        empty={prs.length === 0}
        emptyIcon={GitPullRequest}
        emptyTitle="no pull requests"
        emptyDescription={`no ${state} pull requests in ${repo}`}
      >
        <ul className="flex flex-col">
          {prs.map((pr) => {
            const badge = prBadge(pr)
            const isExpanded = expanded === pr.number
            return (
              <li
                key={pr.number}
                className="border-b border-rule last:border-b-0"
              >
                <div className="flex items-center gap-3 py-2">
                  <button
                    type="button"
                    onClick={() =>
                      setExpanded((cur) =>
                        cur === pr.number ? null : pr.number,
                      )
                    }
                    aria-expanded={isExpanded}
                    title="toggle checks"
                    className="font-mono text-[11px] text-ink-faint hover:text-ink w-14 text-left shrink-0"
                  >
                    #{pr.number}
                  </button>
                  <a
                    href={pr.url}
                    target="_blank"
                    rel="noreferrer"
                    className="flex-1 min-w-0 truncate font-mono text-[13px] text-ink hover:text-accent transition-colors"
                  >
                    {pr.title}
                  </a>
                  <span className="hidden sm:block font-mono text-[11px] lowercase text-ink-faint truncate max-w-44 shrink-0">
                    {pr.author?.login ?? ''}
                    {pr.updatedAt ? ` · ${timeAgoIso(pr.updatedAt)}` : ''}
                  </span>
                  <Badge variant={badge.variant}>{badge.label}</Badge>
                </div>
                {isExpanded ? (
                  <ChecksInline repo={repo} number={pr.number} />
                ) : null}
              </li>
            )
          })}
        </ul>
      </PanelShell>
    </div>
  )
}

function ChecksInline({ repo, number }: { repo: string; number: number }) {
  const fetcher = useCallback(() => listPrChecks(repo, number), [repo, number])
  const { data, loading, error } = useGithubQuery(true, fetcher)
  const checks = data ?? []

  return (
    <div className="pb-2 pl-14">
      {error ? (
        <p className="font-mono text-[11px] lowercase text-alert">{error}</p>
      ) : loading && checks.length === 0 ? (
        <p className="font-mono text-[11px] lowercase text-ink-ghost">
          loading checks...
        </p>
      ) : checks.length === 0 ? (
        <p className="font-mono text-[11px] lowercase text-ink-faint">
          no checks reported
        </p>
      ) : (
        <ul className="flex flex-col gap-1">
          {checks.map((check) => (
            <li
              key={checkKey(check)}
              className="flex items-center gap-2 min-w-0"
            >
              <Badge variant={checkVariant(check.bucket)}>{check.bucket}</Badge>
              {check.link ? (
                <a
                  href={check.link}
                  target="_blank"
                  rel="noreferrer"
                  className="font-mono text-[11px] text-ink-faint hover:text-accent truncate"
                >
                  {checkLabel(check)}
                </a>
              ) : (
                <span className="font-mono text-[11px] text-ink-faint truncate">
                  {checkLabel(check)}
                </span>
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
