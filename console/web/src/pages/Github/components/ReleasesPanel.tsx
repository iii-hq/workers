import { Tag } from 'lucide-react'
import { useCallback } from 'react'
import { Badge } from '@/components/ui/Badge'
import { listReleases, releaseUrl, timeAgoIso } from '@/lib/github'
import { useGithubQuery } from '../hooks/useGithubQuery'
import { PanelShell } from './PanelShell'

interface ReleasesPanelProps {
  repo: string
  enabled: boolean
}

export function ReleasesPanel({ repo, enabled }: ReleasesPanelProps) {
  const fetcher = useCallback(() => listReleases(repo), [repo])
  const { data, loading, error } = useGithubQuery(enabled, fetcher)
  const releases = data ?? []

  return (
    <PanelShell
      loading={loading}
      error={error}
      empty={releases.length === 0}
      emptyIcon={Tag}
      emptyTitle="no releases"
      emptyDescription={`no releases in ${repo}`}
    >
      <ul className="flex flex-col">
        {releases.map((release) => (
          <li
            key={release.tagName}
            className="flex items-center gap-3 py-2 border-b border-rule last:border-b-0"
          >
            <a
              href={releaseUrl(repo, release.tagName)}
              target="_blank"
              rel="noreferrer"
              className="font-mono text-[13px] text-ink hover:text-accent transition-colors shrink-0"
            >
              {release.tagName}
            </a>
            <span className="flex-1 min-w-0 truncate font-mono text-[12px] text-ink-faint">
              {release.name ?? ''}
            </span>
            <span className="hidden sm:block font-mono text-[11px] lowercase text-ink-faint shrink-0">
              {timeAgoIso(release.publishedAt ?? release.createdAt)}
            </span>
            {release.isLatest ? <Badge variant="accent">latest</Badge> : null}
            {release.isDraft ? <Badge variant="warn">draft</Badge> : null}
            {release.isPrerelease ? <Badge>prerelease</Badge> : null}
          </li>
        ))}
      </ul>
    </PanelShell>
  )
}
