/**
 * The github page (#/ext/github): the github worker's read surface — pull
 * requests (with CI check rollups), issues, Actions runs, and releases for a
 * chosen repo, plus org-wide search. Data loads on demand (repo commit / panel
 * switch / refresh) — no polling; GitHub-side state isn't engine-event-driven
 * and gh rate limits are real. Read-only on purpose: mutations stay in agent
 * flows behind the approval gate.
 *
 * The host only mounts this page when the github worker is connected, so there
 * is no presence gate here; a failed `github::*` call surfaces per panel as the
 * "github call failed" alert.
 */

import { Button, EmptyState, type Host, Input } from '@iii-dev/console-ui'
import { useState } from 'react'
import { loadGithubRepo, saveGithubRepo } from './github-data'
import { IssuesPanel } from './IssuesPanel'
import { FolderGit2, type IconProps, RefreshCw } from './icons'
import { ModeToggle } from './ModeToggle'
import { PrsPanel } from './PrsPanel'
import { ReleasesPanel } from './ReleasesPanel'
import { RunsPanel } from './RunsPanel'
import { SearchPanel } from './SearchPanel'

type Panel = 'prs' | 'issues' | 'runs' | 'releases' | 'search'

const PANEL_OPTIONS: { value: Panel; label: string }[] = [
  { value: 'prs', label: 'pull requests' },
  { value: 'issues', label: 'issues' },
  { value: 'runs', label: 'runs' },
  { value: 'releases', label: 'releases' },
  { value: 'search', label: 'search' },
]

const RepoIcon = (p: IconProps) => <FolderGit2 size={28} {...p} />

export function GithubPage({ host }: { host: Host }) {
  const [panel, setPanel] = useState<Panel>('prs')
  const [repoInput, setRepoInput] = useState(loadGithubRepo)
  const [repo, setRepo] = useState(loadGithubRepo)
  const [bump, setBump] = useState(0)

  const commitRepo = () => {
    const next = repoInput.trim()
    setRepo(next)
    saveGithubRepo(next)
  }

  const needsRepo = panel !== 'search'

  return (
    <div className="gh-page">
      <div className="gh-head">
        <div>
          <div className="gh-title">github</div>
          <div className="gh-sub">{repo || 'no repository selected'}</div>
        </div>
        <Button variant="ghost" size="sm" onClick={() => setBump((b) => b + 1)}>
          <RefreshCw size={14} aria-hidden />
          refresh
        </Button>
      </div>

      <div className="gh-controls">
        {needsRepo ? (
          <Input
            className="gh-repo-input"
            value={repoInput}
            onChange={setRepoInput}
            onBlur={commitRepo}
            onKeyDown={(e) => {
              if (e.key === 'Enter') commitRepo()
            }}
            placeholder="owner/name"
            preserveCase
            aria-label="repository"
          />
        ) : null}
        <ModeToggle<Panel>
          value={panel}
          onChange={setPanel}
          options={PANEL_OPTIONS}
          aria-label="github panel"
        />
      </div>

      <div className="gh-body">
        {needsRepo && repo === '' ? (
          <EmptyState
            icon={RepoIcon}
            title="no repository selected"
            description="enter a repository above as owner/name — e.g. iii-hq/workers — and press enter. the search panel works without one."
          />
        ) : (
          // Remount on repo/panel/refresh so every fetch hook restarts clean.
          <div key={`${panel}:${repo}:${bump}`}>
            {panel === 'prs' ? (
              <PrsPanel host={host} repo={repo} enabled />
            ) : panel === 'issues' ? (
              <IssuesPanel host={host} repo={repo} enabled />
            ) : panel === 'runs' ? (
              <RunsPanel host={host} repo={repo} enabled />
            ) : panel === 'releases' ? (
              <ReleasesPanel host={host} repo={repo} enabled />
            ) : (
              <SearchPanel host={host} enabled />
            )}
          </div>
        )}
      </div>
    </div>
  )
}
