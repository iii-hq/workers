/* The Git tool window, laid out like WebStorm's. It docks at the bottom of
   the IDE and has two tabs: Log (the branches, the commit graph and the
   selected commit) and Worktrees. One set of worktree operations serves
   both tabs, so an outcome lands once, in the window's header, and is
   announced once. */

import type { Host, LiveAnnouncement } from '@iii-dev/console-ui'
import { LiveRegion, Tabs, TabsContent, TabsList, TabsTrigger, Toolbar, Tooltip } from '@iii-dev/console-ui'
import { Minus } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { GitLogTab } from './GitLogTab'
import { GitWorktreesTab } from './GitWorktreesTab'
import type { CommitDetails, CommitFile } from './git-log-window'
import { useWorktreeOps, type WorktreesPage } from './use-worktree-ops'
import { DeleteBranchDialog, RemoveWorktreeDialog, WARNING } from './WorktreeForms'

export type GitTab = 'log' | 'worktrees'

export function GitToolWindow({
  host,
  root,
  page,
  tab,
  onTabChange,
  onHide,
  narrow,
  paneKey,
  onOpenCommitFile,
  onOpenCompareFile,
}: {
  host: Host
  root: string
  page: WorktreesPage
  tab: GitTab
  onTabChange(tab: GitTab): void
  onHide(): void
  narrow?: boolean
  /** Keys the window's own layout (pane widths, open groups) to this pane. */
  paneKey: string
  /** Opens a file a commit changed as a diff tab. */
  onOpenCommitFile(file: CommitFile, details: CommitDetails): void
  /** Opens a file as it is at `ref`, beside its working copy. */
  onOpenCompareFile(file: CommitFile, ref: string): void
}) {
  const ops = useWorktreeOps(host, root, page, true, 'view')
  const target = ops.list?.defaultBranch ?? null
  const windowRef = useRef<HTMLDivElement>(null)
  const [announcement, setAnnouncement] = useState<LiveAnnouncement | null>(null)
  const noteSeq = useRef(ops.noteSeq)
  // "Show in Log" from the Worktrees tab: the branch the Log should select.
  const [focusBranch, setFocusBranch] = useState<{ name: string; seq: number } | null>(null)

  // The outcome of what was asked here is announced here; the header chip
  // speaks only for its own.
  useEffect(() => {
    if (ops.noteSeq === noteSeq.current) return
    noteSeq.current = ops.noteSeq
    const text = ops.note
    if (text === null || !ops.noteIsMine) return
    setAnnouncement((previous) => ({
      seq: (previous?.seq ?? 0) + 1,
      text,
      urgency: WARNING.test(text) ? 'assertive' : 'polite',
    }))
  }, [ops.noteSeq, ops.note, ops.noteIsMine])

  // A dialog closes onto the list it was asked from.
  const refocus = () =>
    requestAnimationFrame(() =>
      windowRef.current?.querySelector<HTMLElement>('.shui-git-panel[data-state="active"] [data-git-focus]')?.focus(),
    )

  return (
    <div ref={windowRef} className="shui-git-window" data-narrow={narrow || undefined}>
      <Tabs
        value={tab}
        onValueChange={(value) => onTabChange(value === 'worktrees' ? 'worktrees' : 'log')}
        className="shui-git-tabs-root"
      >
        <Toolbar
          aria-label="Git"
          className="shui-git-bar"
          end={
            <Tooltip label="Hide Git (Shift+Alt+G)">
              <button type="button" className="shui-terminal-action" onClick={onHide} aria-label="Hide Git">
                <Minus aria-hidden />
              </button>
            </Tooltip>
          }
        >
          <span className="shui-git-title">Git</span>
          <TabsList className="shui-git-tabs">
            <TabsTrigger value="log" icon={false}>
              Log
            </TabsTrigger>
            <TabsTrigger value="worktrees" icon={false}>
              Worktrees
            </TabsTrigger>
          </TabsList>
          {ops.note !== null ? (
            <span className={`shui-git-note${WARNING.test(ops.note) ? ' warn' : ''}`} title={ops.note}>
              {ops.note}
            </span>
          ) : null}
        </Toolbar>
        <TabsContent value="log" forceMount className="shui-git-panel">
          <GitLogTab
            host={host}
            root={root}
            ops={ops}
            active={tab === 'log'}
            paneKey={paneKey}
            focusBranch={focusBranch}
            narrow={narrow}
            onOpenCommitFile={onOpenCommitFile}
            onOpenCompareFile={onOpenCompareFile}
          />
        </TabsContent>
        <TabsContent value="worktrees" forceMount className="shui-git-panel">
          <GitWorktreesTab
            root={root}
            ops={ops}
            narrow={narrow}
            onShowInLog={(name) => {
              setFocusBranch((previous) => ({ name, seq: (previous?.seq ?? 0) + 1 }))
              onTabChange('log')
            }}
          />
        </TabsContent>
        <RemoveWorktreeDialog
          removing={ops.removing}
          target={target}
          onConfirm={() => {
            ops.confirmRemove()
            refocus()
          }}
          onCancel={() => {
            ops.cancelRemove()
            refocus()
          }}
        />
        <DeleteBranchDialog
          deleting={ops.deletingBranch}
          target={target}
          onConfirm={() => {
            ops.confirmDeleteBranch()
            refocus()
          }}
          onCancel={() => {
            ops.cancelDeleteBranch()
            refocus()
          }}
        />
        <LiveRegion announcement={announcement} />
      </Tabs>
    </div>
  )
}
