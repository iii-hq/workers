/* The Source Control view's data: the staged and unstaged comparisons of
   the browsed root, the current branch, and the verbs that change them.
   Loaded only while the view is shown, re-read on every git refresh. */

import type { Host } from '@iii-dev/console-ui'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { errorMessage } from '../lib/format'
import { type GitComparisonEntry, gitComparison } from './git'
import {
  gitCommit,
  gitDiscard,
  gitStage,
  gitStageAll,
  gitUnstage,
  gitUnstageAll,
} from './git-actions'

export type SourceControlPhase = 'idle' | 'loading' | 'ready' | 'not-a-repo' | 'error'

export interface SourceControlState {
  phase: SourceControlPhase
  branch: string | null
  staged: readonly GitComparisonEntry[]
  unstaged: readonly GitComparisonEntry[]
  error: string | null
  busy: boolean
  /** The last action's outcome, for a status line. */
  note: string | null
  reload: () => void
  stage: (paths: readonly string[]) => Promise<void>
  stageAll: () => Promise<void>
  unstage: (paths: readonly string[]) => Promise<void>
  unstageAll: () => Promise<void>
  discard: (entries: readonly GitComparisonEntry[]) => Promise<void>
  commit: (message: string) => Promise<boolean>
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
  const [staged, setStaged] = useState<readonly GitComparisonEntry[]>([])
  const [unstaged, setUnstaged] = useState<readonly GitComparisonEntry[]>([])
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [note, setNote] = useState<string | null>(null)
  const seqRef = useRef(0)
  const [reloadEpoch, setReloadEpoch] = useState(0)

  // biome-ignore lint/correctness/useExhaustiveDependencies: the epochs are reload triggers
  useEffect(() => {
    if (!active || root === null) return
    const seq = ++seqRef.current
    setPhase((current) => (current === 'ready' ? current : 'loading'))
    void Promise.all([
      gitComparison(host, root, 'staged'),
      gitComparison(host, root, 'unstaged'),
      currentBranch(host, root),
    ])
      .then(([stagedState, unstagedState, branchName]) => {
        if (seqRef.current !== seq) return
        setBranch(branchName)
        if (stagedState.kind === 'not-a-repo' || unstagedState.kind === 'not-a-repo') {
          setPhase('not-a-repo')
          setStaged([])
          setUnstaged([])
          return
        }
        if (stagedState.kind === 'error' || unstagedState.kind === 'error') {
          setPhase('error')
          setError(stagedState.kind === 'error' ? stagedState.message : unstagedState.kind === 'error' ? unstagedState.message : null)
          return
        }
        setStaged(stagedState.changes)
        setUnstaged(unstagedState.changes)
        setError(null)
        setPhase('ready')
      })
      .catch((err: unknown) => {
        if (seqRef.current !== seq) return
        setPhase('error')
        setError(errorMessage(err))
      })
  }, [host, root, refreshEpoch, reloadEpoch, active])

  // biome-ignore lint/correctness/useExhaustiveDependencies: a new root starts from a blank view
  useEffect(() => {
    setPhase('idle')
    setStaged([])
    setUnstaged([])
    setNote(null)
  }, [root])

  const reload = useCallback(() => setReloadEpoch((value) => value + 1), [])

  const perform = useCallback(
    async (label: string, action: () => Promise<string | undefined>) => {
      if (root === null) return
      setBusy(true)
      setNote(null)
      try {
        const outcome = await action()
        setNote(typeof outcome === 'string' ? outcome : null)
      } catch (err: unknown) {
        setNote(`${label} failed: ${errorMessage(err)}`)
      } finally {
        setBusy(false)
        onChanged()
        setReloadEpoch((value) => value + 1)
      }
    },
    [root, onChanged],
  )

  const stage = useCallback(
    (paths: readonly string[]) => perform('stage', () => gitStage(host, root ?? '', paths).then(() => undefined)),
    [host, root, perform],
  )
  const stageAll = useCallback(
    () => perform('stage', () => gitStageAll(host, root ?? '').then(() => undefined)),
    [host, root, perform],
  )
  const unstage = useCallback(
    (paths: readonly string[]) => perform('unstage', () => gitUnstage(host, root ?? '', paths).then(() => undefined)),
    [host, root, perform],
  )
  const unstageAll = useCallback(
    () => perform('unstage', () => gitUnstageAll(host, root ?? '').then(() => undefined)),
    [host, root, perform],
  )
  const discard = useCallback(
    (entries: readonly GitComparisonEntry[]) =>
      perform('discard', async () => {
        const results = await gitDiscard(host, root ?? '', entries)
        const failed = results.filter((result) => result.error !== null)
        if (failed.length > 0) {
          throw new Error(`${failed[0].path}: ${failed[0].error}${failed.length > 1 ? ` (+${failed.length - 1} more)` : ''}`)
        }
        return `discarded ${results.length} ${results.length === 1 ? 'change' : 'changes'}`
      }),
    [host, root, perform],
  )
  const commit = useCallback(
    async (message: string) => {
      let ok = false
      await perform('commit', async () => {
        const sha = await gitCommit(host, root ?? '', message)
        ok = true
        return `committed ${sha}`
      })
      return ok
    },
    [host, root, perform],
  )

  return useMemo(
    () => ({ phase, branch, staged, unstaged, error, busy, note, reload, stage, stageAll, unstage, unstageAll, discard, commit }),
    [phase, branch, staged, unstaged, error, busy, note, reload, stage, stageAll, unstage, unstageAll, discard, commit],
  )
}
