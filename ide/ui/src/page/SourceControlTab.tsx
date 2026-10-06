/* The Source Control view as a commit panel: a Commit tab
   (every uncommitted change with a tick for the next commit, rollback, the
   commit box with Generate), a Stash tab and a History tab. Only the tab in
   front loads anything. */

import type { Host } from '@iii-dev/console-ui'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@iii-dev/console-ui'
import { GitBranch } from 'lucide-react'
import { useState } from 'react'
import { CommitView } from './CommitView'
import type { DiffSource } from './diff-source'
import { HistoryView } from './HistoryView'
import { StashView } from './StashView'
import { opensFileDirectly } from './scm-view'
import type { SourceControlState } from './use-source-control'

export type ScmTab = 'commit' | 'stash' | 'history'
const TAB_KEY = 'iii::ide::scm-tab'

function readTab(): ScmTab {
  try {
    const stored = window.localStorage.getItem(TAB_KEY)
    return stored === 'stash' || stored === 'history' ? stored : 'commit'
  } catch {
    return 'commit'
  }
}

function writeTab(tab: ScmTab): void {
  try {
    window.localStorage.setItem(TAB_KEY, tab)
  } catch {
    // Storage may be blocked; the live view still switches.
  }
}

interface SourceControlTabProps {
  host: Host
  root: string | null
  conversationId?: string | null
  scm: SourceControlState
  /** Bumped whenever git state may have moved. */
  refreshEpoch: number
  /** The diff tab in front, to mark its row. */
  activeDiff: { path: string; source: DiffSource } | null
  /** Click opens the diff as a preview tab; double click keeps it. */
  onOpenDiff: (path: string, source: DiffSource, pin: boolean) => void
  /** An image has no text diff: its row opens the file. */
  onOpenFile: (path: string) => void
  /** "Compare with…" from a change's menu. */
  onCompare: (path: string) => void
  /** "Show history" from a change's menu: the Git log narrowed to these paths. */
  onShowHistory: (paths: string[]) => void
  /** "Delete…" from a change's menu: the Explorer's delete, so tabs on the file close. */
  onDeleteFile: (path: string) => Promise<void>
  onChanged: () => void
}

export function SourceControlTab({
  host,
  root,
  conversationId,
  scm,
  refreshEpoch,
  activeDiff,
  onOpenDiff,
  onOpenFile,
  onCompare,
  onShowHistory,
  onDeleteFile,
  onChanged,
}: SourceControlTabProps) {
  const [tab, setTab] = useState<ScmTab>(readTab)
  const activeSource = activeDiff?.source ?? null
  const activePath = activeDiff?.path ?? null

  return (
    <Tabs
      className="shui-scm"
      value={tab}
      onValueChange={(next) => {
        const value = next as ScmTab
        setTab(value)
        writeTab(value)
      }}
    >
      <div className="shui-scm-head">
        <TabsList variant="line" aria-label="Source control">
          <TabsTrigger value="commit" icon={false}>
            Commit
          </TabsTrigger>
          <TabsTrigger value="stash" icon={false}>
            Stash
          </TabsTrigger>
          <TabsTrigger value="history" icon={false}>
            History
          </TabsTrigger>
        </TabsList>
        <span className="spacer" />
        {scm.branch ? (
          <span className="shui-scm-branch" title={`On branch ${scm.branch}`}>
            <GitBranch aria-hidden />
            {scm.branch}
          </span>
        ) : null}
      </div>
      <TabsContent value="commit" className="shui-scm-panel">
        <CommitView
          host={host}
          root={root}
          conversationId={conversationId}
          scm={scm}
          activePath={activeSource?.type === 'uncommitted' ? activePath : null}
          onOpenChange={(entry, pin) =>
            opensFileDirectly(entry) ? onOpenFile(entry.path) : onOpenDiff(entry.path, { type: 'uncommitted' }, pin)
          }
          onOpenFile={onOpenFile}
          onCompare={onCompare}
          onShowHistory={onShowHistory}
          onDeleteFile={onDeleteFile}
        />
      </TabsContent>
      <TabsContent value="stash" className="shui-scm-panel">
        <StashView
          host={host}
          root={root}
          refreshEpoch={refreshEpoch}
          activeSource={activeSource}
          activePath={activePath}
          onOpenDiff={onOpenDiff}
          onChanged={onChanged}
        />
      </TabsContent>
      <TabsContent value="history" className="shui-scm-panel">
        <HistoryView
          host={host}
          root={root}
          refreshEpoch={refreshEpoch}
          activeSource={activeSource}
          activePath={activePath}
          onOpenDiff={onOpenDiff}
          onChanged={onChanged}
        />
      </TabsContent>
    </Tabs>
  )
}
