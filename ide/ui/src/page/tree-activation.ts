interface ComposedPathSource {
  composedPath?: () => readonly unknown[]
}

export interface TreeRowEvent extends ComposedPathSource {
  nativeEvent?: ComposedPathSource | null
}

/** Controlled selection mirrors must not re-enter file activation. */
export function shouldActivateTreeSelection(path: string | null, activePath: string | null): boolean {
  return path !== null && path !== activePath
}

interface TreeRowPathCarrier {
  dataset?: { itemPath?: unknown; itemType?: unknown }
  getAttribute?: (name: string) => unknown
}

function readTreeData(
  entry: TreeRowPathCarrier,
  key: 'itemPath' | 'itemType',
  attribute: 'data-item-path' | 'data-item-type',
): string | null {
  const datasetValue = entry.dataset?.[key]
  if (typeof datasetValue === 'string') return datasetValue
  const attributeValue = entry.getAttribute?.(attribute)
  return typeof attributeValue === 'string' ? attributeValue : null
}

/** Resolve the nearest file-tree row from an event crossing its shadow root. */
export function filePathFromTreeEvent(event: TreeRowEvent): string | null {
  const path = event.nativeEvent?.composedPath?.() ?? event.composedPath?.() ?? []
  for (const entry of path) {
    if (typeof entry !== 'object' || entry == null) continue
    const carrier = entry as TreeRowPathCarrier
    const itemPath = readTreeData(carrier, 'itemPath', 'data-item-path')
    if (itemPath == null || itemPath.length === 0) continue
    const itemType = readTreeData(carrier, 'itemType', 'data-item-type')
    if (itemType != null && itemType !== 'file') return null
    return itemPath
  }
  return null
}

/** Re-open a file when the tree suppresses selection change for the active row. */
export function reactivateSelectedFile(
  event: TreeRowEvent,
  selectedPath: string | null,
  activate: (path: string) => void,
): boolean {
  const clickedPath = filePathFromTreeEvent(event)
  if (clickedPath == null || clickedPath !== selectedPath) return false
  activate(clickedPath)
  return true
}

export interface TreeItemRef {
  path: string
  kind: 'file' | 'directory'
}

/** The nearest tree row under an event, file or folder, from the
    composed path across the shadow root. Directory paths keep the
    model's trailing slash. */
export function treeItemFromEvent(event: TreeRowEvent): TreeItemRef | null {
  const path = event.nativeEvent?.composedPath?.() ?? event.composedPath?.() ?? []
  for (const entry of path) {
    if (typeof entry !== 'object' || entry == null) continue
    const carrier = entry as TreeRowPathCarrier
    const itemPath = readTreeData(carrier, 'itemPath', 'data-item-path')
    if (itemPath == null || itemPath.length === 0) continue
    const itemType = readTreeData(carrier, 'itemType', 'data-item-type')
    // @pierre/trees marks a folder row `data-item-type="folder"`.
    return { path: itemPath, kind: itemType === 'folder' ? 'directory' : 'file' }
  }
  return null
}

/** Mark the row a menu acts on, and the rows shown under a folder's, in
    the tree's shadow root (tree-theme styles them); null clears them.
    Resolves to the target row, which the menu opens under. */
export function markTreeMenuRows(root: ParentNode, path: string | null): HTMLElement | null {
  for (const row of root.querySelectorAll('[data-shui-menu]')) row.removeAttribute('data-shui-menu')
  if (path === null) return null
  let target: HTMLElement | null = null
  for (const row of root.querySelectorAll<HTMLElement>('[data-type="item"][data-item-path]')) {
    const rowPath = row.dataset.itemPath ?? ''
    if (rowPath === path) {
      row.setAttribute('data-shui-menu', 'target')
      // A sticky copy of a folder row comes first; the row in the list is the one in place.
      if (row.dataset.fileTreeStickyRow !== 'true') target = row
    } else if (path.endsWith('/') && rowPath.startsWith(path)) row.setAttribute('data-shui-menu', 'scope')
  }
  return target
}
