/* Open file tabs whose file is gone from disk. A tab outlives its file on
   purpose — closing it silently would lose where the user was, and an
   unsaved draft can put the file back — so the page keeps the set of
   root-relative paths it knows are missing and shows it: struck through
   in the tab strip, a "not found" state in the editor. The set is fed by
   the probe after a restore, by the editor's own read failing, and by
   the live change feed; a file coming back (created, or modified after
   an atomic replace) clears its mark. Pure functions over a set; every
   one returns the same instance when nothing changed. */

export type MissingPaths = ReadonlySet<string>

export const NO_MISSING: MissingPaths = new Set()

export interface PathChange {
  rel: string
  /** `created`, `modified`, or `deleted` (the live feed's kinds). */
  kind: string
  dir: boolean
}

function under(path: string, dir: string): boolean {
  return dir === '' || path === dir || path.startsWith(`${dir}/`)
}

/** Mark one path, or clear it. */
export function withMissing(missing: MissingPaths, path: string, isMissing: boolean): MissingPaths {
  if (missing.has(path) === isMissing) return missing
  const next = new Set(missing)
  if (isMissing) next.add(path)
  else next.delete(path)
  return next
}

/** Mark several paths at once (a probe's answer). */
export function withMissingPaths(missing: MissingPaths, paths: readonly string[]): MissingPaths {
  if (paths.every((path) => missing.has(path))) return missing
  const next = new Set(missing)
  for (const path of paths) next.add(path)
  return next
}

/** Apply a burst of disk changes: a deleted file (or folder) marks the open
    tabs under it, a file created or modified clears its mark. */
export function missingAfterChanges(
  missing: MissingPaths,
  changes: readonly PathChange[],
  openFiles: ReadonlySet<string>,
): MissingPaths {
  let next: Set<string> | null = null
  const edit = () => {
    next ??= new Set(missing)
    return next
  }
  for (const change of changes) {
    if (change.kind === 'deleted') {
      if (change.dir) {
        for (const path of openFiles) {
          if (under(path, change.rel) && !(next ?? missing).has(path)) edit().add(path)
        }
      } else if (openFiles.has(change.rel) && !(next ?? missing).has(change.rel)) {
        edit().add(change.rel)
      }
    } else if (!change.dir && (next ?? missing).has(change.rel)) {
      edit().delete(change.rel)
    }
  }
  return next ?? missing
}

/** Forget paths that are no longer open. */
export function pruneMissing(missing: MissingPaths, openFiles: ReadonlySet<string>): MissingPaths {
  let next: Set<string> | null = null
  for (const path of missing) {
    if (!openFiles.has(path)) {
      next ??= new Set(missing)
      next.delete(path)
    }
  }
  return next ?? missing
}

/** From a batch stat: the root-relative paths the worker could not see. */
export function missingFromStats(
  results: readonly { path: string; success: boolean }[],
  root: string,
): string[] {
  const prefix = root.endsWith('/') ? root : `${root}/`
  const gone: string[] = []
  for (const result of results) {
    if (result.success) continue
    gone.push(result.path.startsWith(prefix) ? result.path.slice(prefix.length) : result.path)
  }
  return gone
}
