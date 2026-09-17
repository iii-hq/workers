import { useContainerNarrow as useSharedContainerNarrow } from '@iii-dev/console-ui/hooks'

/**
 * Observe an element's own width and report whether it is currently
 * narrower than `threshold` — the shared `@iii-dev/console-ui/hooks`
 * implementation (ResizeObserver on the node, synchronous first measure,
 * zero widths ignored) in the tuple shape the Console's callers use.
 */
export function useContainerNarrow(
  threshold: number,
): [(node: HTMLElement | null) => void, boolean] {
  const { ref, narrow } = useSharedContainerNarrow({ below: threshold })
  return [ref, narrow]
}
