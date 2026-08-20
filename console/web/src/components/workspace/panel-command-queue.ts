import type { TabScreen } from '@/lib/workspace-tabs'

export type PendingPanelCommand =
  | { type: 'open'; screen: TabScreen; tabId: string }
  | {
      type: 'add'
      tabId: string
      side: 'left' | 'right'
      mobileIndex?: number
    }
  | { type: 'split'; tabId: string }

/**
 * Preserve user intent in FIFO order. Repeated delivery of the same page-open
 * event is the sole coalescing case; panel additions are deliberately kept.
 */
export function enqueuePanelCommand(
  queue: PendingPanelCommand[],
  command: PendingPanelCommand,
): boolean {
  const duplicateOpen =
    command.type === 'open' &&
    queue.some(
      (queued) =>
        queued.type === 'open' &&
        queued.screen === command.screen &&
        queued.tabId === command.tabId,
    )
  if (duplicateOpen) return false
  queue.push(command)
  return true
}
