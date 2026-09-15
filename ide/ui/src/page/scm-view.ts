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
}

/** Preserve metadata; null display paths form a separate absolute-path tree. */
export function buildChangeTree<T extends { path: string }>(
  entries: readonly T[],
  getPath: (entry: T) => string | null = (entry) => entry.path,
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
    const parts = (relativePath ?? entry.path).split('/').filter(Boolean)
    parts.pop()
    for (const name of parts) {
      const path = parent.path === '/' ? `/${name}` : parent.path ? `${parent.path}/${name}` : name
      const key = external ? `\u0000outside:${path}` : path
      let directory = directories.get(key)
      if (!directory) {
        directory = { name, path, directories: [], entries: [] }
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
  return root
}
