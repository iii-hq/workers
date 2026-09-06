/* Root-relative path helpers shared by the explorer, the tabs and the
   search view. Paths are forward-slash, never absolute, never trailing-
   slash (directories in the FileTree model carry the slash; strip it at
   the boundary with `stripDirSlash`). */

export function basename(path: string): string {
  const trimmed = stripDirSlash(path)
  const idx = trimmed.lastIndexOf('/')
  return idx === -1 ? trimmed : trimmed.slice(idx + 1)
}

/** `a/b/c.ts` → `a/b`; `c.ts` → `''`. */
export function dirname(path: string): string {
  const trimmed = stripDirSlash(path)
  const idx = trimmed.lastIndexOf('/')
  return idx === -1 ? '' : trimmed.slice(0, idx)
}

export function stripDirSlash(path: string): string {
  return path.endsWith('/') ? path.slice(0, -1) : path
}

export function joinRel(dir: string, name: string): string {
  return dir === '' ? name : `${dir}/${name}`
}

/** Every proper ancestor directory of `path`, shallowest first. */
export function ancestorDirs(path: string): string[] {
  const segments = stripDirSlash(path).split('/')
  const out: string[] = []
  for (let i = 1; i < segments.length; i++) out.push(segments.slice(0, i).join('/'))
  return out
}

/** `a/b/c.ts` → `['a', 'b', 'c.ts']` with the cumulative path of each. */
export function breadcrumbSegments(path: string): { name: string; path: string }[] {
  const segments = stripDirSlash(path).split('/').filter((s) => s !== '')
  return segments.map((name, index) => ({
    name,
    path: segments.slice(0, index + 1).join('/'),
  }))
}

/** True when `path` is `dir` itself or lives under it. */
export function isUnder(path: string, dir: string): boolean {
  if (dir === '') return true
  const p = stripDirSlash(path)
  return p === dir || p.startsWith(`${dir}/`)
}

/** A name a user typed for a new entry: no separators, no dot segments. */
export function validEntryName(name: string): string | null {
  const trimmed = name.trim()
  if (trimmed === '') return 'a name is required'
  if (trimmed === '.' || trimmed === '..') return 'the name cannot be "." or ".."'
  if (trimmed.includes('/') || trimmed.includes('\\')) return 'the name cannot contain slashes'
  if (trimmed.includes('\0')) return 'the name contains an invalid character'
  return null
}
