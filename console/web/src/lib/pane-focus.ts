/**
 * Focus handoff for a screen the keyboard asked for.
 *
 * Opening a page from the palette, a go-to chord, a deep link or a worker's
 * `panels.open` records the screen here; the workspace panes consume the
 * request once that screen has a pane and move focus into it, so the next
 * keystroke already goes to the page instead of to wherever the pointer was.
 */

let requested: string | null = null

export function requestPaneFocus(screen: string): void {
  requested = screen
}

/** The pending screen, if it is among `screens`; consuming clears it. */
export function takePaneFocusRequest(
  screens: readonly (string | null)[],
): string | null {
  if (requested === null) return null
  if (!screens.includes(requested)) return null
  const screen = requested
  requested = null
  return screen
}

export function clearPaneFocusRequest(): void {
  requested = null
}

/** Move focus into a pane: its own autofocus target first, else the root. */
export function focusPane(root: HTMLElement): void {
  const target = root.querySelector<HTMLElement>('[data-autofocus]') ?? root
  target.focus({ preventScroll: true })
}

/** The pane root that contains `node`, if any. */
export function paneRootOf(node: EventTarget | null): HTMLElement | null {
  const element = node as Element | null
  if (typeof element?.closest !== 'function') return null
  return element.closest<HTMLElement>('[data-workspace-pane-id]')
}
