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
    return { path: itemPath, kind: itemType === 'directory' ? 'directory' : 'file' }
  }
  return null
}
