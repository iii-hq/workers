/* The Stash tab: the repository's stashes newest first, the files the
   selected one records (each opens its diff against the stash's base), and
   Apply / Pop / Unstash… / Drop, all also on a stash's right-click menu
   with Clear and Show diff. "Stash changes…" sets the working tree aside,
   optionally with unversioned files. */

import type { Host } from '@iii-dev/console-ui'
import {
  Button,
  Checkbox,
  ConfirmDialog,
  EmptyState,
  IconButton,
  Skeleton,
  StatusPanel,
  uiClasses,
} from '@iii-dev/console-ui'
import { errorMessage, formatRelative } from '@iii-dev/console-ui/format'
import { Archive, CircleAlert, GitBranch, RefreshCw, Trash2 } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { anchorFromEvent, useContextMenu } from './ContextMenu'
import type { DiffSource } from './diff-source'
import { FileTypeIcon } from './file-type-icon'
import type { GitFileStatus } from './git'
import {
  gitStashApply,
  gitStashBranch,
  gitStashClear,
  gitStashDrop,
  gitStashPush,
  statusLetter,
  statusTitle,
} from './git-actions'
import { type GitStash, gitStashFiles, gitStashList, type StashFile, stashFileSource } from './git-log'
import { basename, dirname } from './paths'
import { stashMenu } from './scm-menus'
import { TextDialog } from './TextDialog'
import { useSpin } from './use-spin'
import { VirtualList } from './VirtualList'

interface StashViewProps {
  host: Host
  root: string | null
  /** Bumped when git state may have moved (a commit, a refresh). */
  refreshEpoch: number
  activeSource: DiffSource | null
  activePath: string | null
  onOpenDiff: (path: string, source: DiffSource, pin: boolean) => void
  onChanged: () => void
}

export type Load<T> = { kind: 'loading' } | { kind: 'ready'; value: T } | { kind: 'error'; message: string }

export function StashView({
  host,
  root,
  refreshEpoch,
  activeSource,
  activePath,
  onOpenDiff,
  onChanged,
}: StashViewProps) {
  const [list, setList] = useState<Load<GitStash[]>>({ kind: 'loading' })
  const [selected, setSelected] = useState<string | null>(null)
  const [files, setFiles] = useState<Load<StashFile[]> | null>(null)
  // A reload retries details that failed to load: the sha they are keyed on
  // comes back the same.
  const [retry, setRetry] = useState(0)
  const filesRef = useRef(files)
  filesRef.current = files
  const [busy, setBusy] = useState(false)
  const [note, setNote] = useState<{ text: string; failed: boolean } | null>(null)
  const [stashOpen, setStashOpen] = useState(false)
  const [includeUntracked, setIncludeUntracked] = useState(false)
  const [unstashFor, setUnstashFor] = useState<GitStash | null>(null)
  const [unstashPop, setUnstashPop] = useState(false)
  const [unstashIndex, setUnstashIndex] = useState(false)
  const [dropFor, setDropFor] = useState<GitStash | null>(null)
  const [clearOpen, setClearOpen] = useState(false)
  const menu = useContextMenu()
  // The stash the open menu acts on, marked in the list while it is open.
  const [menuStash, setMenuStash] = useState<string | null>(null)
  const [epoch, setEpoch] = useState(0)
  // Only the Refresh button spins the icon; reloads after an action don't.
  const [refreshing, setRefreshing] = useState(false)
  const spinning = useSpin(refreshing)
  const seqRef = useRef(0)

  useEffect(() => {
    if (root === null) return
    const seq = ++seqRef.current
    gitStashList(host, root)
      .then((stashes) => {
        if (seqRef.current !== seq) return
        setRefreshing(false)
        setList({ kind: 'ready', value: stashes })
        setSelected((current) => (stashes.some((stash) => stash.sha === current) ? current : (stashes[0]?.sha ?? null)))
        if (filesRef.current?.kind === 'error') setRetry((value) => value + 1)
      })
      .catch((err: unknown) => {
        if (seqRef.current !== seq) return
        setRefreshing(false)
        setList({ kind: 'error', message: errorMessage(err) })
      })
  }, [host, root, refreshEpoch, epoch])

  const stashes = list.kind === 'ready' ? list.value : []
  const current = stashes.find((stash) => stash.sha === selected) ?? null

  // Keyed on the sha: a reload finds the same entry as a new object, and
  // a sha's content never changes.
  const currentSha = current?.sha ?? null
  useEffect(() => {
    if (root === null || current === null) {
      setFiles(null)
      return
    }
    let live = true
    setFiles({ kind: 'loading' })
    gitStashFiles(host, root, current.sha)
      .then((value) => live && setFiles({ kind: 'ready', value }))
      .catch((err: unknown) => live && setFiles({ kind: 'error', message: errorMessage(err) }))
    return () => {
      live = false
    }
  }, [host, root, currentSha, retry])

  const perform = useCallback(
    async (label: string, action: () => Promise<string>) => {
      setBusy(true)
      setNote(null)
      try {
        setNote({ text: await action(), failed: false })
      } catch (err: unknown) {
        setNote({ text: `${label} failed: ${errorMessage(err)}`, failed: true })
      } finally {
        setBusy(false)
        setEpoch((value) => value + 1)
        onChanged()
      }
    },
    [onChanged],
  )

  const apply = (stash: GitStash, pop: boolean) =>
    void perform(pop ? 'pop' : 'apply', async () => {
      await gitStashApply(host, root ?? '', stash.ref, pop)
      return `${pop ? 'popped' : 'applied'} ${stash.ref}`
    })

  const unstash = (stash: GitStash) => {
    setUnstashPop(false)
    setUnstashIndex(false)
    setUnstashFor(stash)
  }

  // A stash's diff tab is one per file: Show diff opens its first.
  const showDiff = async (stash: GitStash, pin: boolean) => {
    setSelected(stash.sha)
    try {
      const [first] = await gitStashFiles(host, root ?? '', stash.sha)
      if (first) onOpenDiff(first.path, stashFileSource(stash, first), pin)
      else setNote({ text: `${stash.ref} changed nothing under this folder`, failed: false })
    } catch (err: unknown) {
      setNote({ text: `show diff failed: ${errorMessage(err)}`, failed: true })
    }
  }

  return (
    <div className="shui-stash">
      <div className="shui-commit-toolbar" role="toolbar" aria-label="Stashes">
        <IconButton label="Stash changes…" disabled={busy || root === null} onClick={() => setStashOpen(true)}>
          <Archive aria-hidden />
        </IconButton>
        <IconButton
          label="Refresh"
          disabled={busy}
          aria-busy={spinning}
          onClick={() => {
            setRefreshing(true)
            setEpoch((value) => value + 1)
          }}
        >
          <RefreshCw aria-hidden className={spinning ? uiClasses.spin : undefined} />
        </IconButton>
        <span className="spacer" />
        {list.kind === 'ready' ? (
          <span className="shui-toolbar-count num">
            {stashes.length} {stashes.length === 1 ? 'stash' : 'stashes'}
          </span>
        ) : null}
      </div>

      {note ? (
        <div className="shui-commit-note pad" data-failed={note.failed || undefined} role="status">
          <span>{note.text}</span>
        </div>
      ) : null}

      <div className="shui-stash-list">
        {list.kind === 'loading' ? (
          <div className="shui-commit-skeleton">
            <Skeleton className="shui-commit-skeleton-row tall" />
            <Skeleton className="shui-commit-skeleton-row tall" />
          </div>
        ) : list.kind === 'error' ? (
          <StatusPanel
            variant="alert"
            icon={<CircleAlert aria-hidden />}
            headline="Couldn't read the stashes"
            detail={list.message}
            action={
              <Button type="button" variant="pill" size="sm" onClick={() => setEpoch((value) => value + 1)}>
                Retry
              </Button>
            }
          />
        ) : stashes.length === 0 ? (
          <EmptyState
            compact
            title="No stashes"
            description="Stash changes to set work aside without committing it."
            action={{ label: 'Stash changes…', onClick: () => setStashOpen(true) }}
          />
        ) : (
          <ul className="shui-log-rows" aria-label="Stashes">
            {stashes.map((stash) => (
              <li key={stash.sha}>
                <div
                  className="shui-log-row"
                  data-selected={stash.sha === selected || undefined}
                  data-menu={(menu.isOpen && menuStash === stash.sha) || undefined}
                >
                  <button
                    type="button"
                    className="shui-log-main"
                    aria-current={stash.sha === selected || undefined}
                    title={stash.message}
                    onClick={() => setSelected(stash.sha)}
                    onContextMenu={(event) => {
                      event.preventDefault()
                      setMenuStash(stash.sha)
                      // Under the row, at the pointer's x: the marked row stays in sight.
                      const row = event.currentTarget.closest('.shui-log-row') ?? event.currentTarget
                      menu.open(
                        { x: anchorFromEvent(event).x, y: row.getBoundingClientRect().bottom },
                        stashMenu(stash, {
                          busy,
                          apply,
                          unstash,
                          drop: setDropFor,
                          clear: () => setClearOpen(true),
                          showDiff: (target, pin) => void showDiff(target, pin),
                        }),
                      )
                    }}
                  >
                    <span className="subject">{stash.message}</span>
                    <span className="meta">
                      <span className="mono">{stash.ref}</span>
                      <span className="num" title={new Date(stash.time * 1000).toLocaleString()}>
                        {formatRelative(stash.time)}
                      </span>
                      {stash.branch ? (
                        <span className="branch">
                          <GitBranch aria-hidden />
                          <span className="mono">{stash.branch}</span>
                        </span>
                      ) : null}
                    </span>
                  </button>
                  <span className="shui-log-actions">
                    <IconButton label={`Drop ${stash.ref}`} disabled={busy} onClick={() => setDropFor(stash)}>
                      <Trash2 aria-hidden />
                    </IconButton>
                  </span>
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>

      {current ? (
        <section className="shui-log-details" aria-label={`${current.ref} contents`}>
          <div className="shui-log-details-head">
            <span className="mono">{current.ref}</span>
            <span className="spacer" />
            {files?.kind === 'ready' ? (
              <span className="num faint">
                {files.value.length} {files.value.length === 1 ? 'file' : 'files'}
              </span>
            ) : null}
          </div>
          <p className="shui-log-details-subject">{current.message}</p>
          <FileList
            files={files}
            isActive={(file) =>
              activePath === file.path &&
              activeSource?.type === 'revision' &&
              activeSource.to === stashFileSource(current, file).to
            }
            onOpen={(file, pin) => onOpenDiff(file.path, stashFileSource(current, file), pin)}
          />
          <div className="shui-log-details-actions">
            <Button type="button" variant="primary" size="sm" disabled={busy} onClick={() => apply(current, false)}>
              Apply
            </Button>
            <Button type="button" variant="pill" size="sm" disabled={busy} onClick={() => apply(current, true)}>
              Pop
            </Button>
            <span className="spacer" />
            <Button type="button" variant="ghost" size="sm" disabled={busy} onClick={() => unstash(current)}>
              Unstash…
            </Button>
          </div>
        </section>
      ) : null}

      <TextDialog
        open={stashOpen}
        title="Stash changes"
        description="Sets the working tree's changes aside and cleans it."
        label="Message"
        placeholder="What these changes are"
        allowEmpty
        confirmLabel="Stash"
        onCancel={() => setStashOpen(false)}
        onConfirm={(message) => {
          setStashOpen(false)
          void perform('stash', async () => {
            await gitStashPush(host, root ?? '', { message, includeUntracked })
            return 'stashed the working tree'
          })
        }}
      >
        <Checkbox
          label="Include unversioned files"
          checked={includeUntracked}
          onChange={(event) => setIncludeUntracked(event.currentTarget.checked)}
        />
      </TextDialog>
      <TextDialog
        open={unstashFor !== null}
        title={unstashFor ? `Unstash ${unstashFor.ref}` : 'Unstash'}
        description="With a branch name, a new branch is checked out where the stash was made and the stash is popped there; the boxes below then do not apply."
        label="As new branch"
        placeholder="Leave empty to unstash here"
        allowEmpty
        confirmLabel="Unstash"
        onCancel={() => setUnstashFor(null)}
        onConfirm={(name) => {
          const stash = unstashFor
          setUnstashFor(null)
          if (!stash) return
          if (name !== '') {
            void perform('branch', async () => {
              await gitStashBranch(host, root ?? '', name, stash.ref)
              return `switched to ${name} with ${stash.ref} applied`
            })
            return
          }
          void perform(unstashPop ? 'pop' : 'apply', async () => {
            await gitStashApply(host, root ?? '', stash.ref, unstashPop, unstashIndex)
            return `${unstashPop ? 'popped' : 'applied'} ${stash.ref}${unstashIndex ? ' with its index' : ''}`
          })
        }}
      >
        <Checkbox
          label="Pop stash"
          checked={unstashPop}
          onChange={(event) => setUnstashPop(event.currentTarget.checked)}
        />
        <Checkbox
          label="Reinstate index"
          checked={unstashIndex}
          onChange={(event) => setUnstashIndex(event.currentTarget.checked)}
        />
      </TextDialog>
      <ConfirmDialog
        open={clearOpen}
        onOpenChange={(open) => (open ? undefined : setClearOpen(false))}
        title="Clear all stashes?"
        description={`Every stash (${stashes.length}) is deleted. This can't be undone.`}
        confirmLabel="Clear"
        tone="danger"
        onCancel={() => setClearOpen(false)}
        onConfirm={() => {
          setClearOpen(false)
          void perform('clear', async () => {
            await gitStashClear(host, root ?? '')
            return 'cleared every stash'
          })
        }}
      />
      {menu.element}
      <ConfirmDialog
        open={dropFor !== null}
        onOpenChange={(open) => (open ? undefined : setDropFor(null))}
        title={dropFor ? `Drop ${dropFor.ref}?` : 'Drop stash?'}
        description="The stashed changes are deleted. This can't be undone."
        details={dropFor ? [dropFor.message] : undefined}
        confirmLabel="Drop"
        tone="danger"
        onCancel={() => setDropFor(null)}
        onConfirm={() => {
          const stash = dropFor
          setDropFor(null)
          if (!stash) return
          void perform('drop', async () => {
            await gitStashDrop(host, root ?? '', stash.ref)
            return `dropped ${stash.ref}`
          })
        }}
      />
    </div>
  )
}

export interface FileListItem {
  path: string
  status: GitFileStatus
  from?: string
  untracked?: boolean
}

/** The files of a stash or a commit; each row opens its diff. */
export function FileList<T extends FileListItem>({
  files,
  isActive,
  onOpen,
}: {
  files: Load<readonly T[]> | null
  isActive: (file: T) => boolean
  onOpen: (file: T, pin: boolean) => void
}) {
  if (files === null || files.kind === 'loading') {
    return (
      <div className="shui-commit-skeleton">
        <Skeleton className="shui-commit-skeleton-row" />
      </div>
    )
  }
  if (files.kind === 'error')
    return (
      <p className="shui-commit-note" data-failed>
        {files.message}
      </p>
    )
  if (files.value.length === 0) return <p className="shui-log-details-empty">No file changes under this folder.</p>
  // Windowed: a commit or stash of thousands of files mounts only the rows in view.
  return (
    <VirtualList
      rows={files.value}
      rowHeight={28}
      rowKey={(file) => file.path}
      className="shui-log-files"
      role="list"
      aria-label="Files"
      renderRow={(file) => (
        // biome-ignore lint/a11y/useSemanticElements: a <li> cannot sit in VirtualList's row
        <div role="listitem">
          <button
            type="button"
            className="shui-log-file"
            data-selected={isActive(file) || undefined}
            title={file.from ? `${file.from} → ${file.path}` : file.path}
            onClick={() => onOpen(file, false)}
            onDoubleClick={() => onOpen(file, true)}
          >
            <FileTypeIcon path={file.path} className="file-icon" />
            <span className="name">{basename(file.path)}</span>
            <span className="dir">{dirname(file.path)}</span>
            <span className="shui-ctree-status" data-status={file.status} title={statusTitle(file.status)}>
              {statusLetter(file.status)}
            </span>
          </button>
        </div>
      )}
    />
  )
}
