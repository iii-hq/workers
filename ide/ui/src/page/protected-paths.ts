/* The worker's protected paths (`code.non_accessible_globs`, e.g. `.env`,
   keys): listed in the explorer but never read or written, by agents or by
   the IDE. A read of one fails exactly like a missing file (C211 folds the
   two so a path never leaks), so the page tells them apart by the globs
   `coder::info` reports. */

const SPECIAL = /[.+^${}()|[\]\\]/g

/** A glob as the worker matches it: `**` any run of folders (zero
    included), `*` anything within one name, `?` one character. */
export function globToRegExp(glob: string): RegExp {
  let source = ''
  for (let i = 0; i < glob.length; i += 1) {
    const char = glob[i]
    if (char === '*' && glob[i + 1] === '*') {
      // `**/` may match no folder at all.
      if (glob[i + 2] === '/') {
        source += '(?:.*/)?'
        i += 2
      } else {
        source += '.*'
        i += 1
      }
    } else if (char === '*') {
      source += '[^/]*'
    } else if (char === '?') {
      source += '[^/]'
    } else {
      source += char.replace(SPECIAL, '\\$&')
    }
  }
  return new RegExp(`^${source}$`)
}

/** The path as the worker matches it: relative to the first base path
    holding it, else (unjailed, outside every base path) the absolute path
    without its leading slash. */
export function workerRelative(abs: string, basePaths: readonly string[]): string {
  for (const base of basePaths) {
    const prefix = base.endsWith('/') ? base : `${base}/`
    if (abs.startsWith(prefix)) return abs.slice(prefix.length)
  }
  return abs.replace(/^\/+/, '')
}

/** Whether a file of the IDE's folder `root` (by its path below it) is one
    of the protected ones, matched the way the worker matches it. */
export function isProtectedPath(
  rel: string | null,
  globs: readonly string[],
  root: string | null,
  basePaths: readonly string[],
): boolean {
  if (rel === null || rel === '' || root === null || globs.length === 0) return false
  const path = workerRelative(root.endsWith('/') ? `${root}${rel}` : `${root}/${rel}`, basePaths)
  return path !== '' && globs.some((glob) => globToRegExp(glob).test(path))
}
