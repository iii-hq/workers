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

/** Whether a root-relative path is one of the protected ones.
    ponytail: matched against the IDE-root-relative path; a glob anchored
    to another base path (no leading `**`) can miss. */
export function isProtectedPath(rel: string | null, globs: readonly string[]): boolean {
  if (rel === null || rel === '') return false
  return globs.some((glob) => globToRegExp(glob).test(rel))
}
