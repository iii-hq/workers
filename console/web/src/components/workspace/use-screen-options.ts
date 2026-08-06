import { useConversationsCtx } from '@/lib/conversations-context'
import { buildViewOptions } from '@/lib/nav-options'
import { useExtPages } from '@/lib/ui-slots'
import {
  CHAT_SCREEN,
  screenForExtPage,
  type TabScreen,
} from '@/lib/workspace-tabs'

export interface ScreenOption {
  value: TabScreen
  label: string
}

/**
 * Every screen a workspace tab can attach: the chat view, the available
 * first-party pages (optional-worker pages appear only while their worker
 * is present), and the worker-injected pages. Configuration is absent by
 * design — console settings open as an overlay page, not a tab screen.
 * Must render under `ConversationsProvider`.
 */
export function useScreenOptions(): {
  screenOptions: ScreenOption[]
  extPageTitles: ReadonlyMap<string, string>
} {
  const {
    worktreeAvailable,
    browserAvailable,
    memoryAvailable,
    githubAvailable,
  } = useConversationsCtx()
  const extPages = useExtPages()
  const screenOptions: ScreenOption[] = [
    { value: CHAT_SCREEN, label: 'chat' },
    ...buildViewOptions(
      worktreeAvailable,
      browserAvailable,
      memoryAvailable,
      githubAvailable,
    ),
    ...extPages.map((page) => ({
      value: screenForExtPage(page.id),
      label: page.title,
    })),
  ]
  const extPageTitles = new Map(extPages.map((page) => [page.id, page.title]))
  return { screenOptions, extPageTitles }
}
