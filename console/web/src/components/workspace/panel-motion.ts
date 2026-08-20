export type PanelMotionDirection = 'left' | 'right'

/** Panels leave toward their nearest workspace edge. */
export function panelMotionDirection(
  column: number,
  columns: number,
): PanelMotionDirection {
  return column < (columns - 1) / 2 ? 'left' : 'right'
}

/**
 * A pane owns the divider immediately before it, except the first pane,
 * which owns the divider immediately after it.
 */
export function dividerForPanel(
  column: number,
  columns: number,
): number | null {
  if (columns <= 1 || column < 0 || column >= columns) return null
  return column === 0 ? 1 : column
}
