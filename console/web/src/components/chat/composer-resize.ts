export interface ComposerEditorSize {
  width: number
  height: number
}

export type ComposerResizeKind =
  | 'initial'
  | 'content'
  | 'container'
  | 'unchanged'

const SIZE_EPSILON_PX = 0.5

/**
 * Height changes from typing should animate. Width-driven reflow comes from
 * pane/sidebar resize and must stay attached to that direct manipulation.
 */
export function classifyComposerResize(
  previous: ComposerEditorSize | null,
  next: ComposerEditorSize,
): ComposerResizeKind {
  if (previous === null) return 'initial'
  if (Math.abs(next.width - previous.width) > SIZE_EPSILON_PX) {
    return 'container'
  }
  if (Math.abs(next.height - previous.height) > SIZE_EPSILON_PX) {
    return 'content'
  }
  return 'unchanged'
}
