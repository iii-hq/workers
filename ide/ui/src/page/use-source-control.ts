/* The Commit panel's data: every uncommitted change of the browsed root
   (HEAD → working copy, no staged/unstaged split), which of
   them are ticked for the next commit, the current branch, and the verbs
   that act on them. Tracked changes start ticked and unversioned files
   start unticked; a tick survives reloads for as long as the path stays
   changed. Loaded only while the view is shown, re-read on every git refresh. */

import type { Host } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { changeSummary, entryPaths } from './commit-tree'
import { type GitComparisonEntry, gitComparison } from './git'
import { gitCommitChanges, gitDiscard, gitPush, gitStashPush } from './git-actions'

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
}

interface ExecResponse {
  exit_code: number | null
  stdout: string
}

async function currentBranch(host: Host, root: string): Promise<string | null> {
  try {
    const out = await host.iii.trigger<ExecResponse>('shell::exec', {
      command: 'git',
      args: ['rev-parse', '--abbrev-ref', 'HEAD'],
      cwd: root,
      timeout_ms: 10_000,
    })
    if (out.exit_code !== 0) return null
    const name = out.stdout.trim()
    return name === '' ? null : name
  } catch {
    return null
  }
}

export function useSourceControl(
  host: Host,
  root: string | null,
  refreshEpoch: number,
  active: boolean,
  onChanged: () => void,
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
  const [reloadEpoch, setReloadEpoch] = useState(0)
  const [refreshing, setRefreshing] = useState(false)

  // biome-ignore lint/correctness/useExhaustiveDependencies: the epochs are reload triggers
  useEffect(() => {
    if (!active || root === null) return
    const seq = ++seqRef.current
    setPhase((current) => (current === 'ready' ? current : 'loading'))
    void Promise.all([gitComparison(host, root, 'uncommitted'), currentBranch(host, root)])
      .then(([state, branchName]) => {
        if (seqRef.current !== seq) return
        setRefreshing(false)
        setBranch(branchName)
        if (state.kind === 'not-a-repo') {
          setPhase('not-a-repo')
          setAll([])
          return
        }
        if (state.kind === 'error') {
          setPhase('error')
          setError(state.message)
          return
        }
        setAll(state.changes)
        // Forget ticks for paths that are no longer changed.
        const live = new Set(state.changes.map((change) => change.path))
        setExcluded((current) => new Set([...current].filter((path) => live.has(path))))
        setAdopted((current) => new Set([...current].filter((path) => live.has(path))))
        setError(null)
        setPhase('ready')
      })
      .catch((err: unknown) => {
        if (seqRef.current !== seq) return
        setRefreshing(false)
        setPhase('error')
        setError(errorMessage(err))
      })
  }, [host, root, refreshEpoch, reloadEpoch, active])

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
    setReloadEpoch((value) => value + 1)
  }, [])

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
        onChanged()
        setReloadEpoch((value) => value + 1)
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
    ],
  )
}
