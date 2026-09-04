/* Loading the two sides of a diff tab. Each source resolves its own pair
   of bodies; the page caches results per tab id and asks again when the
   disk (or the index) moved. Git sides come through `shell::exec` in argv
   form, the working copy through `coder::read-file`, turn sides through
   the worker's change history, recorded changes through
   `coder::change-diff`. */

import type { Host } from '@iii-dev/console-ui'
import { coderReadFile, joinPath } from './coder'
import { type DiffSource, diffSourceLabel } from './diff-source'
import { gitHeadBaseline } from './git'
import { EDITOR_FULL_READ_BUDGET } from './large-file'
import { imageMimeFromPath } from './file-kinds'
import { fetchSessionTurn, relativeToRoot, type SessionTurn, type TurnFileRecord, type TurnPreImage } from './turns'

/** What a diff tab says above the diff, or instead of it: a headline in
    plain words, one line on what that means for the reader, and whether it
    warns (the diff shows more or less than the source promises) or merely
    informs. */
export interface DiffNote {
  headline: string
  detail?: string
  tone?: 'neutral' | 'warn'
}

const IMAGE_NOTE: DiffNote = {
  headline: 'This is an image',
  detail: 'Images have no text diff. Open the file to view it.',
}

export interface DiffContents {
  oldContents: string
  newContents: string
  /** A caveat worth a row above the diff, or the reason there is none. */
  note?: DiffNote
  /** Neither side can be shown as text. */
  binary?: boolean
  /** The old side is not available at all: nothing to diff. */
  noBaseline?: boolean
  /** Working-copy identity, for a later "open in editor" that wants it. */
  worktreeRevision?: string
}

interface ExecResponse {
  exit_code: number | null
  stdout: string
  stderr: string
  stdout_truncated: boolean
}

/** `git show <spec>` as text; null when the path is absent at that spec
    (a file added since, or removed by then). */
async function gitSide(host: Host, root: string, spec: string): Promise<string | null> {
  const out = await host.iii.trigger<ExecResponse>('shell::exec', {
    command: 'git',
    args: ['show', spec],
    cwd: root,
    timeout_ms: 15_000,
  })
  if (out.exit_code !== 0) {
    const detail = out.stderr.trim()
    if (/exists on disk, but not in|does not exist in|exists in the index, but not at|but not in the index/.test(detail)) {
      return null
    }
    if (/invalid object name|unknown revision|bad revision|not a valid object|ambiguous argument/i.test(detail)) {
      throw new Error(`unknown revision: ${spec.split(':')[0]}`)
    }
    throw new Error(detail || `git show exited ${out.exit_code}`)
  }
  if (out.stdout_truncated) throw new Error('the committed body is larger than the shell output cap')
  if (out.stdout.includes('\0') || out.stdout.includes('�')) throw new Error('binary file')
  return out.stdout
}

/** The working copy as text; null when the file is gone. */
export async function worktreeSide(
  host: Host,
  root: string,
  path: string,
): Promise<{ contents: string | null; revision?: string }> {
  try {
    const out = await coderReadFile(host, joinPath(root, path), { maxOutputBytes: EDITOR_FULL_READ_BUDGET })
    if (out.is_utf8 === false) throw new Error('binary file')
    if (out.more_lines === true) throw new Error('the working copy is larger than the diff budget')
    return { contents: out.content ?? '', revision: out.revision ?? undefined }
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    if (message.includes('C211') || message.includes('not found or not accessible')) return { contents: null }
    throw error
  }
}

function imageOrText(path: string, oldSide: string | null, newSide: string | null, extra: Partial<DiffContents> = {}): DiffContents {
  if (imageMimeFromPath(path) !== null) {
    return { oldContents: '', newContents: '', binary: true, note: IMAGE_NOTE, ...extra }
  }
  return { oldContents: oldSide ?? '', newContents: newSide ?? '', ...extra }
}

/** The stored pre-image as a body: the text when whole, '' when the
    path did not exist, null when the body is unavailable (over the cap,
    binary, pruned). */
export function preImageBody(image: TurnPreImage | null | undefined): string | null {
  if (!image) return null
  if (image.missing) return ''
  if (image.binary || image.truncated) return null
  return typeof image.content === 'string' ? image.content : null
}

/** A turn's record for one root-relative path. */
export function turnFileFor(turn: SessionTurn, root: string, rel: string): TurnFileRecord | null {
  return turn.files.find((file) => relativeToRoot(file.path, root) === rel) ?? null
}

export async function loadTurnDiff(
  host: Host,
  root: string,
  rel: string,
  turn: SessionTurn,
): Promise<DiffContents> {
  const file = turnFileFor(turn, root, rel)
  if (file === null) {
    return {
      oldContents: '',
      newContents: '',
      noBaseline: true,
      note: {
        headline: 'This turn did not touch the file',
        detail: 'The turn kept no record of this path, so there is nothing to compare.',
      },
    }
  }
  if (imageMimeFromPath(rel) !== null) {
    return { oldContents: '', newContents: '', binary: true, note: IMAGE_NOTE }
  }
  let note: DiffNote | undefined
  let oldSide: string | null
  if (file.before == null && file.kind === 'created') {
    // A creation the watcher saw: no stored pre-image, but the file did
    // not exist before the turn.
    oldSide = ''
  } else {
    oldSide = preImageBody(file.before)
    if (oldSide === null) {
      // Last resort: the committed body. Any edit made before the turn is
      // then attributed to it, so say so.
      const slash = rel.lastIndexOf('/')
      const cwd = slash === -1 ? root : joinPath(root, rel.slice(0, slash))
      const committed = await gitHeadBaseline(host, cwd, slash === -1 ? rel : rel.slice(slash + 1))
      if (committed === null) {
        return {
          oldContents: '',
          newContents: '',
          noBaseline: true,
          note: {
            headline: 'Nothing to compare against',
            detail: 'The body before this turn was not kept and the file is not committed, so the old side is gone.',
            tone: 'warn',
          },
        }
      }
      oldSide = committed
      note = {
        headline: 'Compared against the last commit',
        detail: 'The body before this turn was not kept, so edits made since the commit but before the turn show up here too.',
        tone: 'warn',
      }
    }
  }
  let newSide: string | null
  let worktreeRevision: string | undefined
  if (file.kind === 'deleted') {
    newSide = ''
  } else if (file.after) {
    newSide = preImageBody(file.after)
    if (newSide === null) {
      const current = await worktreeSide(host, root, rel)
      newSide = current.contents
      worktreeRevision = current.revision
      note = note ?? {
        headline: 'Showing the working copy',
        detail: 'The body after this turn was not kept, so edits made since the turn show up here too.',
        tone: 'warn',
      }
    }
  } else {
    const current = await worktreeSide(host, root, rel)
    newSide = current.contents
    worktreeRevision = current.revision
  }
  return { oldContents: oldSide, newContents: newSide ?? '', note, worktreeRevision }
}

export interface ChangeDiffResponse {
  path: string
  old_contents?: string
  new_contents?: string
  is_binary: boolean
}

/** Resolve a diff tab's two sides. `turns` answers turn sources from a
    per-turn cache the page keeps (one `shell::turns::get` per turn). */
export async function loadDiffContents(
  host: Host,
  root: string,
  path: string,
  source: DiffSource,
  turns: { get(turnId: string): Promise<SessionTurn | null> },
): Promise<DiffContents> {
  switch (source.type) {
    case 'staged': {
      const [head, index] = await Promise.all([
        gitSide(host, root, `HEAD:./${path}`).catch((error: unknown) => {
          // An unborn HEAD has nothing to show on the old side.
          if (error instanceof Error && /unknown revision/.test(error.message)) return null
          throw error
        }),
        gitSide(host, root, `:./${path}`),
      ])
      return imageOrText(path, head, index)
    }
    case 'unstaged': {
      const [index, current] = await Promise.all([gitSide(host, root, `:./${path}`), worktreeSide(host, root, path)])
      // An untracked file has no index side: everything reads as added.
      return imageOrText(path, index, current.contents, { worktreeRevision: current.revision })
    }
    case 'compare': {
      const [ref, current] = await Promise.all([gitSide(host, root, `${source.ref}:./${path}`), worktreeSide(host, root, path)])
      return imageOrText(path, ref, current.contents, {
        worktreeRevision: current.revision,
        note:
          ref === null
            ? {
                headline: `Not in ${diffSourceLabel(source)}`,
                detail: 'The file does not exist at that revision, so the whole working copy reads as added.',
              }
            : undefined,
      })
    }
    case 'turn': {
      const turn = await turns.get(source.turnId)
      if (turn === null) {
        return {
          oldContents: '',
          newContents: '',
          noBaseline: true,
          note: {
            headline: 'This turn is no longer in the change history',
            detail: 'Its record could not be read from the worker: it may have been pruned, or the read failed.',
            tone: 'warn',
          },
        }
      }
      return loadTurnDiff(host, root, path, turn)
    }
    case 'change': {
      const out = await host.iii.trigger<ChangeDiffResponse>('coder::change-diff', { change_id: source.changeId })
      if (out.is_binary) {
        return {
          oldContents: '',
          newContents: '',
          binary: true,
          note: { headline: 'This is a binary file', detail: 'There is no text diff to show.' },
        }
      }
      return { oldContents: out.old_contents ?? '', newContents: out.new_contents ?? '' }
    }
  }
}

/** One `shell::turns::get` per turn, shared by every diff tab that reads it. */
export function createTurnCache(host: Host, sessionId: string | null | undefined) {
  const cache = new Map<string, Promise<SessionTurn | null>>()
  return {
    get(turnId: string): Promise<SessionTurn | null> {
      if (!sessionId) return Promise.resolve(null)
      let pending = cache.get(turnId)
      if (!pending) {
        pending = fetchSessionTurn(host, sessionId, turnId).catch(() => null)
        cache.set(turnId, pending)
      }
      return pending
    },
    forget(turnId: string) {
      cache.delete(turnId)
    },
    clear() {
      cache.clear()
    },
  }
}
