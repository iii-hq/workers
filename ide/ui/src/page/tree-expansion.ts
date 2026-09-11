import type { FileTree, FileTreeDirectoryHandle } from '@pierre/trees'

export function expandedDirectoryPaths(
  model: Pick<FileTree, 'getItem'>,
  directoryPaths: Iterable<string>,
): string[] {
  const paths = [...directoryPaths]
  const knownPaths = new Set(paths)
  const expandedPaths = new Set<string>()

  for (const path of paths) {
    const handle = model.getItem(path) ?? model.getItem(`${path}/`)
    if (
      handle?.isDirectory() &&
      (handle as FileTreeDirectoryHandle).isExpanded()
    ) {
      expandedPaths.add(path)
    }
  }

  return paths.filter((path) => {
    if (!expandedPaths.has(path)) return false

    const segments = path.split('/')
    for (let end = 1; end < segments.length; end++) {
      const ancestor = segments.slice(0, end).join('/')
      if (knownPaths.has(ancestor) && !expandedPaths.has(ancestor)) return false
    }
    return true
  })
}
