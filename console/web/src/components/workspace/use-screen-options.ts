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
 * Every screen a workspace tab can attach: the chat view, the first-party
 * pages, and the worker-injected pages (whose presence already tracks
 * worker connectedness via trigger GC). Configuration is absent by
 * design — console settings open as an overlay page, not a tab screen.
 */
export function useScreenOptions(): {
  screenOptions: ScreenOption[]
  extPageTitles: ReadonlyMap<string, string>
} {
  const extPages = useExtPages()
  const screenOptions: ScreenOption[] = [
    { value: CHAT_SCREEN, label: 'chat' },
    ...buildViewOptions(),
    ...extPages.map((page) => ({
      value: screenForExtPage(page.id),
      label: page.title,
    })),
  ]
  const extPageTitles = new Map(extPages.map((page) => [page.id, page.title]))
  return { screenOptions, extPageTitles }
}
