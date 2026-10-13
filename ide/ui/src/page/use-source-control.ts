/* The Commit panel's data: every uncommitted change of the browsed root
   (HEAD → working copy, no staged/unstaged split), which of
   them are ticked for the next commit, the current branch, and the verbs
   that act on them. Tracked changes start ticked and unversioned files
   start unticked; a tick survives reloads for as long as the path stays
   changed. Loaded only while the view is shown, from the page's own git
   status. */

import type { Host } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { changeSummary, entryPaths } from './commit-tree'
import { type GitComparisonEntry, type GitState, gitUncommittedFrom } from './git'
import { gitCommitChanges, gitDiscard, gitIgnore, gitLocalPatch, gitPush, gitStage, gitStashPush } from './git-actions'

export type SourceControlPhase = 'idle' | 'loading' | 'ready' | 'not-a-repo' | 'error'

export interface CommitOptions {
  message: string
  amend: boolean
  signOff: boolean
  noVerify: boolean
  author: string
  push: boolean
}

export interface SourceControlState {
  phase: SourceControlPhase
  branch: string | null
  /** Tracked changes (modified, added, deleted, renamed). */
  changes: readonly GitComparisonEntry[]
  /** Untracked files. */
  unversioned: readonly GitComparisonEntry[]
  /** The ticked entries, changes first. */
  included: readonly GitComparisonEntry[]
  isIncluded: (entry: GitComparisonEntry) => boolean
  setIncluded: (entries: readonly GitComparisonEntry[], included: boolean) => void
  error: string | null
  /** A refresh the user asked for is in flight (background re-reads don't count). */
  refreshing: boolean
  busy: boolean
  /** The last action's outcome, for a status line. */
  note: { text: string; failed: boolean } | null
  reload: () => void
  /** Undo the working-tree changes of tracked entries. */
  rollback: (entries: readonly GitComparisonEntry[], options: { keepAdded: boolean }) => Promise<void>
  /** Commit the ticked entries; resolves true on success. */
  commit: (options: CommitOptions) => Promise<boolean>
  /** Set these entries aside in a new stash; the rest of the working tree stays. */
  stash: (entries: readonly GitComparisonEntry[], message: string) => Promise<boolean>
  /** `git add` these unversioned entries. */
  add: (entries: readonly GitComparisonEntry[]) => Promise<boolean>
  /** List root-relative paths (a folder ending in `/`) in the root's .gitignore. */
  ignore: (paths: readonly string[]) => Promise<boolean>
  /** Runs `action` with the panel's busy state; what it resolves to (or
      `label failed: …`) is the status line, and the page reads its status
      again afterwards. Resolves true on success. */
  run: (label: string, action: () => Promise<string>) => Promise<boolean>
  /** These entries' changes as one patch, handed to `use`; what it resolves
      to is the status line. Nothing in the working tree moves. */
  patch: (entries: readonly GitComparisonEntry[], use: (patch: string) => Promise<string>) => Promise<void>
}

const files = (count: number) => `${count} ${count === 1 ? 'file' : 'files'}`

/** The fields a derive can change; the content sources follow from them. */
function sameEntries(a: readonly GitComparisonEntry[], b: readonly GitComparisonEntry[]): boolean {
  return (
    a.length === b.length &&
    a.every((entry, index) => {
      const other = b[index]
      return (
        entry.path === other.path &&
        entry.status === other.status &&
        entry.from === other.from &&
        entry.renameFrom === other.renameFrom &&
        entry.staged === other.staged &&
        entry.x === other.x &&
        entry.y === other.y &&
        entry.before.kind === other.before.kind &&
        entry.after.kind === other.after.kind
      )
    })
  )
}

/** `paths` without the ones no longer changed; the same set when none left. */
function onlyLive(paths: ReadonlySet<string>, live: ReadonlySet<string>): ReadonlySet<string> {
  for (const path of paths) if (!live.has(path)) return new Set([...paths].filter((kept) => live.has(kept)))
  return paths
}

/** `page` is the page's own git status from `gitChanges` and the call that
    reads it again, which bumps `refreshEpoch` while the view is active
    unless asked to be quiet. The panel derives its view from that status
    instead of reading git itself, again on a new status object and on
    every `refreshEpoch`: its own reads (HEAD's diff) may have failed, or
    moved with file bytes the status does not carry. `onChanged` is
    expected to read the page's status again. */
export function useSourceControl(
  host: Host,
  root: string | null,
  refreshEpoch: number,
  active: boolean,
  onChanged: () => void,
  page: { git: GitState | null; refresh: (options?: { quiet: boolean }) => Promise<unknown> },
): SourceControlState {
  const [phase, setPhase] = useState<SourceControlPhase>('idle')
  const [branch, setBranch] = useState<string | null>(null)
  const [all, setAll] = useState<readonly GitComparisonEntry[]>([])
  // Tracked paths the user unticked, unversioned paths the user ticked:
  // everything else follows the default, so a new change needs no bookkeeping.
  const [excluded, setExcluded] = useState<ReadonlySet<string>>(new Set())
  const [adopted, setAdopted] = useState<ReadonlySet<string>>(new Set())
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [note, setNote] = useState<SourceControlState['note']>(null)
  const seqRef = useRef(0)
  const [refreshing, setRefreshing] = useState(false)
  const { git: pageGit, refresh: refreshPage } = page

  // biome-ignore lint/correctness/useExhaustiveDependencies: the epoch is a reload trigger
  useEffect(() => {
    // Every run supersedes the read in flight, even one that reads nothing:
    // the page clears its status on a root switch, and a derive of the last
    // root's status must not land on the new root.
    const seq = ++seqRef.current
    if (!active || root === null || pageGit === null) return
    setPhase((current) => (current === 'ready' ? current : 'loading'))
    void gitUncommittedFrom(host, root, pageGit)
      .then((state) => {
        if (seqRef.current !== seq) return
        setRefreshing(false)
        if (state.kind === 'not-a-repo') {
          setBranch(null)
          setPhase('not-a-repo')
          setAll([])
          return
        }
        // A failed read keeps the branch: the commit box starts a fresh
        // message when the branch changes.
        if (state.kind === 'error') {
          setPhase('error')
          setError(state.message)
          return
        }
        if (pageGit.kind === 'ready') setBranch(pageGit.status.branch)
        // An unchanged derive keeps every identity, so a refresh that moved
        // nothing renders nothing: thousands of rows would redraw otherwise.
        setAll((current) => (sameEntries(current, state.changes) ? current : state.changes))
        // Forget ticks for paths that are no longer changed.
        const live = new Set(state.changes.map((change) => change.path))
        setExcluded((current) => onlyLive(current, live))
        setAdopted((current) => onlyLive(current, live))
        setError(null)
        setPhase('ready')
      })
      .catch((err: unknown) => {
        if (seqRef.current !== seq) return
        setRefreshing(false)
        setPhase('error')
        setError(errorMessage(err))
      })
  }, [host, root, pageGit, refreshEpoch, active])

  // Opening the view reads the page's status again: git's own writes (an
  // add or a commit made in a terminal) touch only .git, which the
  // workspace watch does not report. Quietly: the view's tabs just loaded
  // on mounting, and a changed status reaches this one as a new object.
  // A status still null is a root's first read, already in flight.
  // biome-ignore lint/correctness/useExhaustiveDependencies: only opening the view reads again
  useEffect(() => {
    if (active && pageGit !== null) void refreshPage({ quiet: true })
  }, [active])

  // biome-ignore lint/correctness/useExhaustiveDependencies: a new root starts from a blank view
  useEffect(() => {
    setPhase('idle')
    setAll([])
    setExcluded(new Set())
    setAdopted(new Set())
    setNote(null)
  }, [root])

  const changes = useMemo(() => all.filter((entry) => entry.status !== 'untracked'), [all])
  const unversioned = useMemo(() => all.filter((entry) => entry.status === 'untracked'), [all])
  const isIncluded = useCallback(
    (entry: GitComparisonEntry) => (entry.status === 'untracked' ? adopted.has(entry.path) : !excluded.has(entry.path)),
    [adopted, excluded],
  )
  const included = useMemo(
    () => [...changes.filter(isIncluded), ...unversioned.filter(isIncluded)],
    [changes, unversioned, isIncluded],
  )
  const setIncluded = useCallback((entries: readonly GitComparisonEntry[], on: boolean) => {
    const tracked = entries.filter((entry) => entry.status !== 'untracked').map((entry) => entry.path)
    const untracked = entries.filter((entry) => entry.status === 'untracked').map((entry) => entry.path)
    const apply = (set: ReadonlySet<string>, paths: string[], add: boolean) => {
      const next = new Set(set)
      for (const path of paths) {
        if (add) next.add(path)
        else next.delete(path)
      }
      return next
    }
    if (tracked.length > 0) setExcluded((current) => apply(current, tracked, !on))
    if (untracked.length > 0) setAdopted((current) => apply(current, untracked, on))
  }, [])

  const reload = useCallback(() => {
    setRefreshing(true)
    // The refresh bumps the epoch, so the view derives again even when the
    // page's status comes back unchanged.
    void refreshPage()
  }, [refreshPage])

  const perform = useCallback(
    async (label: string, action: () => Promise<string>): Promise<boolean> => {
      if (root === null) return false
      setBusy(true)
      setNote(null)
      try {
        setNote({ text: await action(), failed: false })
        return true
      } catch (err: unknown) {
        setNote({ text: `${label} failed: ${errorMessage(err)}`, failed: true })
        return false
      } finally {
        setBusy(false)
        // onChanged reads the page's status again, and the view derives
        // from what it reads.
        onChanged()
      }
    },
    [root, onChanged],
  )

  const rollback = useCallback(
    async (entries: readonly GitComparisonEntry[], options: { keepAdded: boolean }) => {
      await perform('rollback', async () => {
        const results = await gitDiscard(host, root ?? '', entries, options)
        const failed = results.filter((result) => result.error !== null)
        if (failed.length > 0) {
          throw new Error(
            `${failed[0].path}: ${failed[0].error}${failed.length > 1 ? ` (+${failed.length - 1} more)` : ''}`,
          )
        }
        return `rolled back ${results.length} ${results.length === 1 ? 'file' : 'files'}`
      })
    },
    [host, root, perform],
  )

  const commit = useCallback(
    (options: CommitOptions) =>
      perform(options.push ? 'commit and push' : 'commit', async () => {
        const sha = await gitCommitChanges(host, root ?? '', {
          message: options.message,
          paths: entryPaths(included),
          amend: options.amend,
          signOff: options.signOff,
          noVerify: options.noVerify,
          author: options.author,
        })
        const what = changeSummary(included)
        const done = `${options.amend ? 'amended' : 'committed'} ${sha}${what ? ` · ${what}` : ''}`
        if (!options.push) return done
        try {
          await gitPush(host, root ?? '')
        } catch (err: unknown) {
          // The commit stands; only the push needs another go.
          throw new Error(`${done}, but the push failed: ${errorMessage(err)}`)
        }
        return `${done} · pushed`
      }),
    [host, root, included, perform],
  )

  const stash = useCallback(
    (entries: readonly GitComparisonEntry[], message: string) =>
      perform('stash', async () => {
        await gitStashPush(host, root ?? '', {
          message,
          // A ticked unversioned file is only stashed with --include-untracked.
          includeUntracked: entries.some((entry) => entry.status === 'untracked'),
          paths: entryPaths(entries),
        })
        return `stashed ${entries.length} ${entries.length === 1 ? 'file' : 'files'}`
      }),
    [host, root, perform],
  )

  const add = useCallback(
    (entries: readonly GitComparisonEntry[]) =>
      perform('add', async () => {
        await gitStage(host, root ?? '', entryPaths(entries))
        return `added ${files(entries.length)} to git`
      }),
    [host, root, perform],
  )

  const ignore = useCallback(
    (paths: readonly string[]) =>
      perform('ignore', async () => {
        const added = await gitIgnore(host, root ?? '', paths)
        return added === 0
          ? '.gitignore already lists them'
          : `added ${added} ${added === 1 ? 'line' : 'lines'} to .gitignore`
      }),
    [host, root, perform],
  )

  const patch = useCallback(
    async (entries: readonly GitComparisonEntry[], use: (patch: string) => Promise<string>) => {
      if (root === null) return
      setNote(null)
      try {
        const tracked = entries.filter((entry) => entry.status !== 'untracked')
        const untracked = entries.filter((entry) => entry.status === 'untracked').map((entry) => entry.path)
        setNote({ text: await use(await gitLocalPatch(host, root, entryPaths(tracked), untracked)), failed: false })
      } catch (err: unknown) {
        setNote({ text: `patch failed: ${errorMessage(err)}`, failed: true })
      }
    },
    [host, root],
  )

  return useMemo(
    () => ({
      phase,
      branch,
      changes,
      unversioned,
      included,
      isIncluded,
      setIncluded,
      error,
      refreshing,
      busy,
      note,
      reload,
      rollback,
      commit,
      stash,
      add,
      ignore,
      run: perform,
      patch,
    }),
    [
      phase,
      branch,
      changes,
      unversioned,
      included,
      isIncluded,
      setIncluded,
      error,
      refreshing,
      busy,
      note,
      reload,
      rollback,
      commit,
      stash,
      add,
      ignore,
      perform,
      patch,
    ],
  )
}
