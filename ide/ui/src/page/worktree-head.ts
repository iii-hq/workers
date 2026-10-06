/* The branch the chat's folder is on, as the composer's worktree switcher
   shows it: three quick gits, read again when anything could have moved it. */

import type { Host } from '@iii-dev/console-ui'
import { useEffect, useState } from 'react'
import { git } from './git-actions'
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

// Focus moving within the page re-reads at most this often.
const REFOCUS_MS = 3_000
// A read younger than this answers for a new one.
const SHARE_MS = 1_000

// The last read of each folder, shared: the IDE's header and the chat's
// composer often show the same one, and each re-reads on the same events.
const reads = new Map<string, { at: number; epoch: number; head: Promise<Head | null | undefined> }>()

/** The head of `dir`; `undefined` when the read could not run. A read of it
    started less than `maxAge` ms ago answers instead, unless a worktree
    operation has ended since (`epoch` moved). */
export function readHead(host: Host, dir: string, epoch: number, maxAge: number): Promise<Head | null | undefined> {
  const last = reads.get(dir)
  if (last !== undefined && last.epoch === epoch && Date.now() - last.at < maxAge) return last.head
  const head = Promise.all([
    git(host, dir, ['symbolic-ref', '-q', 'HEAD'], 10_000),
    git(
      host,
      dir,
      ['rev-parse', '--show-toplevel', '--absolute-git-dir', '--path-format=absolute', '--git-common-dir'],
      10_000,
    ),
    // Read-only: no index.lock, which a git writing there would fail on.
    git(host, dir, ['--no-optional-locks', 'status', '--porcelain', '--untracked-files=normal'], 10_000),
  ]).then(
    (out) => parseHead(...out),
    () => undefined,
  )
  reads.set(dir, { at: Date.now(), epoch, head })
  return head
}

function sameHead(a: Head | null, b: Head | null): boolean {
  if (a === null || b === null) return a === b
  return a.branch === b.branch && a.worktree === b.worktree && a.linked === b.linked && a.dirty === b.dirty
}

/** The branch `dir` is on. It is read again whenever `refreshKey` changes
    (a turn starting or ending), a worktree operation ends, the page regains
    focus or comes back into view, and when focus moves within the page,
    since a `git switch` in the IDE's terminal beside the chat fires nothing
    else. A read of the folder less than a second old, from any chip, stands
    in for a new one. Only the newest read lands, and until it does the last
    one stands. */
export function useHead(host: Host, dir: string | null, refreshKey: string): Head | null {
  const [probe, setProbe] = useState<{ dir: string; head: Head | null } | null>(null)
  const epoch = useWorktreeEpoch()

  useEffect(() => {
    if (dir === null) return
    let cancelled = false
    let latest = 0
    let lastRead = 0
    const read = async () => {
      const mine = ++latest
      lastRead = Date.now()
      const head = await readHead(host, dir, epoch, SHARE_MS)
      // A read that could not run (the engine hiccuped) says nothing about
      // the folder: the last head stands. Nor does one that finds what the
      // last one did re-render the chip.
      if (cancelled || mine !== latest || head === undefined) return
      setProbe((last) => (last?.dir === dir && sameHead(last.head, head) ? last : { dir, head }))
    }
    // Coming back to the tab fires both focus and visibilitychange: one read.
    const reread = () => {
      if (document.visibilityState === 'visible') void read()
    }
    // A branch switched right after a read is still read once the wait ends.
    // The wait counts from this chip's own last read: another chip's read
    // of the folder on the same focus must not hold this one back, and
    // readHead shares that read with it anyway.
    let trailing: ReturnType<typeof setTimeout> | null = null
    const refocus = () => {
      const wait = lastRead + REFOCUS_MS - Date.now()
      if (wait <= 0) void read()
      else if (trailing === null) {
        trailing = setTimeout(() => {
          trailing = null
          void read()
        }, wait)
      }
    }
    void read()
    window.addEventListener('focus', reread)
    document.addEventListener('visibilitychange', reread)
    document.addEventListener('focusin', refocus)
    return () => {
      cancelled = true
      if (trailing !== null) clearTimeout(trailing)
      window.removeEventListener('focus', reread)
      document.removeEventListener('visibilitychange', reread)
      document.removeEventListener('focusin', refocus)
    }
  }, [host, dir, refreshKey, epoch])

  // While a new folder is read the last head stays, so the switcher (its
  // open menu or dialog) does not vanish in between; a finished read that
  // finds no repository is what hides it.
  return dir === null ? null : (probe?.head ?? null)
}
