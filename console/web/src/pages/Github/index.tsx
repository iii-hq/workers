import { AlertCircle, FolderGit2, RefreshCw } from 'lucide-react'
import { useState } from 'react'
import { Button } from '@/components/ui/Button'
import { EmptyState } from '@/components/ui/EmptyState'
import { Input } from '@/components/ui/Input'
import { ModeToggle } from '@/components/ui/ModeToggle'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { isGithubAvailable, useGithubStatus } from '@/hooks/use-github-status'
import { useConversationsCtx } from '@/lib/conversations-context'
import { loadGithubRepo, saveGithubRepo } from '@/lib/github'
import { IssuesPanel } from './components/IssuesPanel'
import { PrsPanel } from './components/PrsPanel'
import { ReleasesPanel } from './components/ReleasesPanel'
import { RunsPanel } from './components/RunsPanel'
import { SearchPanel } from './components/SearchPanel'

/**
 * The github worker's read surface: pull requests (with CI check rollups),
 * issues, Actions runs, and releases for a chosen repo, plus org-wide
 * search. Data loads on demand (repo commit / panel switch / refresh) —
 * no polling, GitHub-side state isn't engine-event-driven and gh rate
 * limits are real. The nav entry is gated on worker presence; a direct
 * #/github hit with the worker absent lands on the install notice below.
 * Read-only on purpose: mutations stay in agent flows behind the
 * approval gate.
 */

type Panel = 'prs' | 'issues' | 'runs' | 'releases' | 'search'

const PANEL_OPTIONS: { value: Panel; label: string }[] = [
  { value: 'prs', label: 'pull requests' },
  { value: 'issues', label: 'issues' },
  { value: 'runs', label: 'runs' },
  { value: 'releases', label: 'releases' },
  { value: 'search', label: 'search' },
]

export function Github() {
  const { backend } = useConversationsCtx()
  const status = useGithubStatus(backend.id === 'real')
  const available = isGithubAvailable(status)

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
    <main
      className="flex-1 flex flex-col min-h-0 overflow-hidden"
      aria-label="github"
    >
      <header className="shrink-0 px-4 sm:px-6 lg:px-8 py-4 border-b border-rule flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="font-mono text-[16px] font-semibold tracking-[-0.01em] text-ink lowercase">
            github
          </h1>
          <p className="font-mono text-[12px] text-ink-faint mt-0.5 lowercase">
            {available
              ? repo || 'no repository selected'
              : 'worker not connected'}
          </p>
        </div>
        {available ? (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setBump((b) => b + 1)}
            className="gap-1.5"
          >
            <RefreshCw className="w-3.5 h-3.5" aria-hidden />
            refresh
          </Button>
        ) : null}
      </header>

      {available ? (
        <div className="shrink-0 px-4 sm:px-6 lg:px-8 py-3 border-b border-rule flex flex-wrap items-center gap-3">
          {needsRepo ? (
            <Input
              value={repoInput}
              onChange={setRepoInput}
              onBlur={commitRepo}
              onKeyDown={(e) => {
                if (e.key === 'Enter') commitRepo()
              }}
              placeholder="owner/name"
              preserveCase
              aria-label="repository"
              className="w-64 max-w-full"
            />
          ) : null}
          <ModeToggle<Panel>
            value={panel}
            onChange={setPanel}
            options={PANEL_OPTIONS}
          />
        </div>
      ) : null}

      <div className="flex-1 overflow-auto px-4 sm:px-6 lg:px-8 py-4">
        {!available ? (
          status.loading ? (
            <p className="font-mono text-[12px] lowercase text-ink-ghost">
              checking for the github worker...
            </p>
          ) : (
            <StatusPanel
              variant="info"
              icon={<AlertCircle className="w-full h-full" />}
              headline="github worker not installed"
              detail="this page needs the optional github worker (and the gh CLI on its host). run: iii worker add github"
            />
          )
        ) : needsRepo && repo === '' ? (
          <EmptyState
            icon={FolderGit2}
            title="no repository selected"
            description="enter a repository above as owner/name — e.g. iii-hq/workers — and press enter. the search panel works without one."
          />
        ) : (
          // Remount on repo/panel/refresh so every fetch hook restarts clean.
          <div key={`${panel}:${repo}:${bump}`}>
            {panel === 'prs' ? (
              <PrsPanel repo={repo} enabled={available} />
            ) : panel === 'issues' ? (
              <IssuesPanel repo={repo} enabled={available} />
            ) : panel === 'runs' ? (
              <RunsPanel repo={repo} enabled={available} />
            ) : panel === 'releases' ? (
              <ReleasesPanel repo={repo} enabled={available} />
            ) : (
              <SearchPanel enabled={available} />
            )}
          </div>
        )}
      </div>
    </main>
  )
}
