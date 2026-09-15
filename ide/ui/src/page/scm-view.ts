export type ScmViewMode = 'list' | 'tree'
const STORAGE_KEY = 'iii::ide::scm-view-mode'

export function readScmViewMode(key = STORAGE_KEY): ScmViewMode {
  try {
    return window.localStorage.getItem(key) === 'tree' ? 'tree' : 'list'
  } catch {
    return 'list'
  }
}

export function writeScmViewMode(mode: ScmViewMode, key = STORAGE_KEY): void {
  try {
    window.localStorage.setItem(key, mode)
  } catch {
    // Storage may be blocked; the live view can still switch.
  }
}

export interface ChangeDirectory<T extends { path: string }> {
  name: string
  path: string
  directories: ChangeDirectory<T>[]
  entries: T[]
  title?: string
}

/** Display-only POSIX path; never use this value for file actions. */
export function relativeDisplayPath(path: string, root: string): string {
  if (!path.startsWith('/') || !root.startsWith('/')) return path
  const target = path.split('/').filter(Boolean)
  const base = root.split('/').filter(Boolean)
  let common = 0
  while (common < target.length && common < base.length && target[common] === base[common]) common++
  return [...base.slice(common).map(() => '..'), ...target.slice(common)].join('/') || '.'
}

/** Preserve metadata; null display paths form a separate absolute-path tree. */
export function buildChangeTree<T extends { path: string }>(
  entries: readonly T[],
  getPath: (entry: T) => string | null = (entry) => entry.path,
  outsideRoot?: string,
): ChangeDirectory<T> {
  const root: ChangeDirectory<T> = { name: '', path: '', directories: [], entries: [] }
  const directories = new Map<string, ChangeDirectory<T>>([['', root]])
  let outside: ChangeDirectory<T> | undefined
  for (const entry of entries) {
    const relativePath = getPath(entry)
    const external = relativePath === null
    let parent = root
    if (external) {
      if (!outside) {
        outside = { name: 'Outside workspace', path: '/', directories: [], entries: [] }
        root.directories.push(outside)
        directories.set('\u0000outside', outside)
      }
      parent = outside
    }
    const displayPath = relativePath ?? (outsideRoot ? relativeDisplayPath(entry.path, outsideRoot) : entry.path)
    const parts = displayPath.split('/').filter(Boolean)
    const absoluteParts = outsideRoot?.split('/').filter(Boolean) ?? []
    parts.pop()
    for (const name of parts) {
      if (name === '..') absoluteParts.pop()
      else absoluteParts.push(name)
      const path = parent.path === '/' ? `/${name}` : parent.path ? `${parent.path}/${name}` : name
      const key = external ? `\u0000outside:${path}` : path
      let directory = directories.get(key)
      if (!directory) {
        directory = { name, path, directories: [], entries: [] }
        if (external && outsideRoot) directory.title = '/' + absoluteParts.join('/')
        directories.set(key, directory)
        parent.directories.push(directory)
      }
      parent = directory
    }
    parent.entries.push(entry)
  }
  for (const directory of directories.values()) {
    directory.directories.sort((a, b) => a.name.localeCompare(b.name))
    directory.entries.sort((a, b) => (getPath(a) ?? a.path).localeCompare(getPath(b) ?? b.path))
  }
  if (outside && outsideRoot) {
    const compact = (directory: ChangeDirectory<T>): void => {
      while (directory.entries.length === 0 && directory.directories.length === 1) {
        const child = directory.directories[0]
        directory.name += '/' + child.name
        directory.path = child.path
        directory.title = child.title
        directory.entries = child.entries
        directory.directories = child.directories
      }
      directory.directories.forEach(compact)
    }
    outside.directories.forEach(compact)
  }
  return root
}
