/**
 * Pure fixed-height virtualization math.
 *
 * Given the current scroll position, container height, and row height,
 * derive a `[startIndex, endIndex)` slice of items to render plus the
 * vertical offset of that slice inside the height-spacer parent.
 *
 * Used by `WaterfallChart` so a 4000-span trace doesn't materialize
 * 4000 DOM rows on every render. The chart already has fixed
 * `ROW_HEIGHT=32`, a scroll container ref tracking `scrollPosition`,
 * and `containerHeight` from a resize observer, so all this function
 * does is the index arithmetic.
 *
 * Pure on purpose:
 *   - no React imports
 *   - no DOM access
 *   - easy to unit-test the six edge cases (empty list, first paint,
 *     mid-scroll, scroll past end, list shorter than viewport,
 *     negative scroll from browser overshoot).
 */

export interface VirtualWindow {
  /** First index to render (inclusive). */
  startIndex: number
  /** Last index to render (exclusive). */
  endIndex: number
  /** Pixel offset to apply via `transform: translateY(...)` on the slice container. */
  offsetY: number
}

export interface ComputeVirtualWindowArgs {
  /** Current `scrollTop` of the container, in pixels. */
  scrollTop: number
  /** Visible height of the container, in pixels. */
  containerHeight: number
  /** Fixed height per row, in pixels. Must be > 0. */
  rowHeight: number
  /** Total number of items in the source list. */
  itemCount: number
  /**
   * Rows rendered above and below the viewport to absorb fast scrolls
   * without exposing blank space at the seams. Defaults to 8.
   */
  overscan?: number
}

const DEFAULT_OVERSCAN = 8

/**
 * Compute the slice of items that should be rendered for the current
 * scroll position. See module docstring for the contract.
 *
 * First-paint fallback: when `containerHeight` is 0 (the resize observer
 * hasn't fired yet on the very first render), this returns a small
 * window starting at index 0 so the user doesn't see a flash of empty
 * content while the observer settles. The size of that window is
 * `2 * overscan` rows, capped by `itemCount`.
 */
export function computeVirtualWindow(args: ComputeVirtualWindowArgs): VirtualWindow {
  const { scrollTop, containerHeight, rowHeight, itemCount } = args
  const overscan = args.overscan ?? DEFAULT_OVERSCAN

  if (itemCount <= 0 || rowHeight <= 0) {
    return { startIndex: 0, endIndex: 0, offsetY: 0 }
  }

  // First paint: container hasn't been measured yet. Render a minimal
  // top-of-list window so something is on screen while the resize
  // observer settles. Without this the user sees an empty scroll
  // surface for one frame after the data arrives.
  if (containerHeight <= 0) {
    const endIndex = Math.min(itemCount, Math.max(1, overscan * 2))
    return { startIndex: 0, endIndex, offsetY: 0 }
  }

  const clampedScrollTop = Math.max(0, scrollTop)
  const rawStart = Math.floor(clampedScrollTop / rowHeight) - overscan
  const rawEnd = Math.ceil((clampedScrollTop + containerHeight) / rowHeight) + overscan

  const startIndex = Math.max(0, Math.min(itemCount, rawStart))
  const endIndex = Math.max(startIndex, Math.min(itemCount, rawEnd))
  const offsetY = startIndex * rowHeight

  return { startIndex, endIndex, offsetY }
}
