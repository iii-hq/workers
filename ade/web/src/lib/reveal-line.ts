/** The nearest ancestor that scrolls vertically, or null when the document does. */
export function scrollParentOf(
  element: HTMLElement | null,
): HTMLElement | null {
  let node = element?.parentElement ?? null
  while (node) {
    const overflowY = getComputedStyle(node).overflowY
    if (
      (overflowY === 'auto' || overflowY === 'scroll') &&
      node.scrollHeight > node.clientHeight
    ) {
      return node
    }
    node = node.parentElement
  }
  return null
}

/** Scroll offset that centers a line, given the editor host's offset inside
    the scroller and the line's offset inside the host. */
export function centeredScrollTop(
  hostTop: number,
  lineTop: number,
  clientHeight: number,
  lineHeight: number,
): number {
  return Math.max(0, hostTop + lineTop - clientHeight / 2 + lineHeight / 2)
}

export function clampLine(line: number, lineCount: number): number {
  if (!Number.isFinite(line)) return 1
  return Math.min(Math.max(1, Math.floor(line)), Math.max(1, lineCount))
}
