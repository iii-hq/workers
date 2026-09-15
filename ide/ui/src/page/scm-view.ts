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

/** Preserve caller metadata; null paths stay at the root (outside-workspace files). */
export function buildChangeTree<T extends { path: string }>(
  entries: readonly T[],
  getPath: (entry: T) => string | null = (entry) => entry.path,
): ChangeDirectory<T> {
  const root: ChangeDirectory<T> = { name: '', path: '', directories: [], entries: [] }
  const directories = new Map<string, ChangeDirectory<T>>([['', root]])
  for (const entry of entries) {
    const parts = (getPath(entry) ?? '').split('/')
    parts.pop()
    let parent = root
    for (const name of parts) {
      const path = parent.path ? `${parent.path}/${name}` : name
      let directory = directories.get(path)
      if (!directory) {
        directory = { name, path, directories: [], entries: [] }
        directories.set(path, directory)
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
