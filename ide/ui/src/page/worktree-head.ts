/* The branch the chat's folder is on, as the composer's worktree switcher
   shows it: three quick gits, read again when the ide worker says the
   repository moved (`shell::git-changed`), and only the dirty mark read
   again when the folder's files change (`shell::changed`). Nothing is read
   on a timer or on focus. */

import type { Host } from '@iii-dev/console-ui'
import { useEffect, useRef, useState } from 'react'
import { git } from './git-actions'
import { generation, watchGit, watchWorktree } from './git-watch'
import { useWorktreeEpoch } from './use-worktree-ops'

export interface Head {
  /** The checked-out branch; `null` when HEAD is detached. */
  branch: string | null
  /** Top of the worktree the folder sits in. */
  worktree: string
  /** A linked worktree (`git worktree add`) rather than the main checkout. */
  linked: boolean
  /** Uncommitted changes, untracked files included. */
  dirty: boolean
}

interface Exec {
  exit_code: number | null
  stdout: string
}

/** What `symbolic-ref -q HEAD`, the path `rev-parse` and `status` print.
    symbolic-ref exits 1 on a detached HEAD, and works in a repository with
    no commits yet, where `rev-parse --abbrev-ref HEAD` fails; it also never
    prints `heads/<x>` when a tag shares the branch's name. Git before 2.31
    echoes the unknown `--path-format=absolute` where the common dir should
    be, so only an absolute common dir can make the worktree a linked one. */
export function parseHead(ref: Exec | null, paths: Exec | null, status: Exec | null): Head | null {
  if (!ref || !paths || paths.exit_code !== 0 || (ref.exit_code !== 0 && ref.exit_code !== 1)) return null
  const [worktree, gitDir, commonDir] = paths.stdout.trim().split('\n')
  if (!worktree || !gitDir || !commonDir) return null
  const name = ref.exit_code === 0 ? ref.stdout.trim() : ''
  return {
    branch: name.replace(/^refs\/heads\//, '') || null,
    worktree,
    linked: commonDir.startsWith('/') && commonDir !== gitDir,
    dirty: status?.exit_code === 0 && status.stdout.trim() !== '',
  }
}

// A read younger than this answers for a new one.
const SHARE_MS = 1_000

const STATUS_ARGS = ['--no-optional-locks', 'status', '--porcelain', '--untracked-files=normal']

interface Shared<T> {
  at: number
  /** What the read is valid for: the epoch and the change generations. */
  stamp: string
  value: Promise<T>
}

// The last read of each folder, shared: the IDE's header and the chat's
// composer often show the same one, and each re-reads on the same events.
const reads = new Map<string, Shared<Head | null | undefined>>()
const dirtyReads = new Map<string, Shared<boolean | undefined>>()

function shared<T>(
  cache: Map<string, Shared<T>>,
  dir: string,
  stamp: string,
  maxAge: number,
  read: () => Promise<T>,
): Promise<T> {
  const last = cache.get(dir)
  if (last !== undefined && last.stamp === stamp && Date.now() - last.at < maxAge) return last.value
  const value = read()
  cache.set(dir, { at: Date.now(), stamp, value })
  return value
}

/** The head of `dir`; `undefined` when the read could not run. A read of it
    started less than `maxAge` ms ago answers instead, unless a worktree
    operation has ended since (`epoch` moved) or a change was heard since. */
export function readHead(host: Host, dir: string, epoch: number, maxAge: number): Promise<Head | null | undefined> {
  const stamp = `${epoch}:${generation(host, 'git', dir)}:${generation(host, 'worktree', dir)}`
  return shared(reads, dir, stamp, maxAge, () =>
    Promise.all([
      git(host, dir, ['symbolic-ref', '-q', 'HEAD'], 10_000),
      git(
        host,
        dir,
        ['rev-parse', '--show-toplevel', '--absolute-git-dir', '--path-format=absolute', '--git-common-dir'],
        10_000,
      ),
      // Read-only: no index.lock, which a git writing there would fail on.
      git(host, dir, STATUS_ARGS, 10_000),
    ]).then(
      (out) => parseHead(...out),
      () => undefined,
    ),
  )
}

/** Only whether `dir`'s worktree has uncommitted changes: what its files
    changing can move. `undefined` when the read could not run. */
export function readDirty(host: Host, dir: string, maxAge: number): Promise<boolean | undefined> {
  const stamp = `${generation(host, 'git', dir)}:${generation(host, 'worktree', dir)}`
  return shared(dirtyReads, dir, stamp, maxAge, () =>
    git(host, dir, STATUS_ARGS, 10_000).then(
      (out) => (out.exit_code === 0 ? out.stdout.trim() !== '' : undefined),
      () => undefined,
    ),
  )
}

function sameHead(a: Head | null, b: Head | null): boolean {
  if (a === null || b === null) return a === b
  return a.branch === b.branch && a.worktree === b.worktree && a.linked === b.linked && a.dirty === b.dirty
}

/** Only `index` moved: the branch and the worktree stand, the dirty mark may not. */
const onlyIndex = (changes: readonly string[]) => changes.length > 0 && changes.every((change) => change === 'index')

/** The branch `dir` is on. Read when the folder is shown, when `refreshKey`
    changes (a turn starting or ending), when a worktree operation ends, and
    whenever the ide worker reports the repository moved — a `git switch`
    in a terminal included. The folder's files changing reads only the
    dirty mark. A folder in no repository is not read again until one
    appears there. A read of the folder less than a second old, from any
    chip, with no change heard since, stands in for a new one. Only the
    newest read lands, and until it does the last one stands. */
export function useHead(host: Host, dir: string | null, refreshKey: string): Head | null {
  const [probe, setProbe] = useState<{ dir: string; head: Head | null } | null>(null)
  const epoch = useWorktreeEpoch()
  const epochRef = useRef(epoch)
  epochRef.current = epoch
  // This folder's reader, set by the folder's effect below.
  const readRef = useRef<((what: 'head' | 'dirty') => void) | null>(null)
  const shown = dir !== null && probe?.dir === dir ? probe : null
  // The last read found no repository: nothing but one appearing reads again.
  const outsideRef = useRef(false)
  outsideRef.current = shown !== null && shown.head === null
  const inRepo = shown !== null && shown.head !== null

  useEffect(() => {
    if (dir === null) return
    let cancelled = false
    let seq = 0
    let headWanted = 0
    let dirtyWanted = 0
    let headLanded = 0
    let dirtyLanded = 0
    const read = (what: 'head' | 'dirty') => {
      const mine = ++seq
      if (what === 'head') {
        headWanted = mine
        void readHead(host, dir, epochRef.current, SHARE_MS).then((head) => {
          // A read that could not run (the engine hiccuped) says nothing
          // about the folder: the last head stands. Nor does one that finds
          // what the last one did re-render the chip.
          if (cancelled || mine !== headWanted || head === undefined) return
          headLanded = mine
          // A dirty read that started after this one and landed first knows better.
          const keepDirty = dirtyLanded > mine
          setProbe((last) => {
            const lastHead = last?.dir === dir ? last.head : null
            const next = head !== null && keepDirty && lastHead !== null ? { ...head, dirty: lastHead.dirty } : head
            return last?.dir === dir && sameHead(last.head, next) ? last : { dir, head: next }
          })
        })
        return
      }
      dirtyWanted = mine
      void readDirty(host, dir, SHARE_MS).then((dirty) => {
        if (cancelled || mine !== dirtyWanted || mine < headLanded || dirty === undefined) return
        dirtyLanded = mine
        setProbe((last) =>
          last?.dir !== dir || last.head === null || last.head.dirty === dirty
            ? last
            : { dir, head: { ...last.head, dirty } },
        )
      })
    }
    readRef.current = read
    // In no repository the worker watches the folder for a `.git`, and says
    // `repository` once one appears.
    const offGit = watchGit(host, dir, (event) => read(onlyIndex(event.changes) ? 'dirty' : 'head'))
    return () => {
      cancelled = true
      readRef.current = null
      offGit()
    }
  }, [host, dir])

  // The files under the folder: only in a repository can they make it dirty.
  useEffect(() => {
    if (dir === null || !inRepo) return
    return watchWorktree(host, dir, () => readRef.current?.('dirty'))
  }, [host, dir, inRepo])

  // Read when the folder is shown, a turn starts or ends (refreshKey) and a
  // worktree operation ends (epoch).
  useEffect(() => {
    if (dir === null) return
    // A folder already found outside any repository waits for the worker
    // to say one appeared; a turn or a worktree operation does not make one.
    if (outsideRef.current) return
    readRef.current?.('head')
  }, [dir, refreshKey, epoch])

  // While a new folder is read the last head stays, so the switcher (its
  // open menu or dialog) does not vanish in between; a finished read that
  // finds no repository is what hides it.
  return dir === null ? null : (probe?.head ?? null)
}
