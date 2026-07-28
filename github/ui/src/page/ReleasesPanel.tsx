import { Badge, type Host } from '@iii-dev/console-ui'
import { useCallback } from 'react'
import { listReleases, releaseUrl, timeAgoIso } from './github-data'
import { type IconProps, Tag } from './icons'
import { PanelShell } from './PanelShell'
import { useGithubRead } from './useGithubRead'

const ReleaseIcon = (p: IconProps) => <Tag size={28} {...p} />

interface ReleasesPanelProps {
  host: Host
  repo: string
  enabled: boolean
}

export function ReleasesPanel({ host, repo, enabled }: ReleasesPanelProps) {
  const fetcher = useCallback(() => listReleases(host, repo), [host, repo])
  const { data, loading, error } = useGithubRead(enabled, fetcher)
  const releases = data ?? []

  return (
    <PanelShell
      loading={loading}
      error={error}
      empty={releases.length === 0}
      emptyIcon={ReleaseIcon}
      emptyTitle="no releases"
      emptyDescription={`no releases in ${repo}`}
    >
      <ul className="gh-list">
        {releases.map((release) => (
          <li key={release.tagName} className="gh-row">
            <a
              href={releaseUrl(repo, release.tagName)}
              target="_blank"
              rel="noreferrer"
              className="gh-row-tag"
            >
              {release.tagName}
            </a>
            <span className="gh-row-title gh-row-title-quiet">
              {release.name ?? ''}
            </span>
            <span className="gh-row-meta">
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
