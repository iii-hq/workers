/**
 * The guided tour that follows setup lives in the `onboarding` worker: its
 * page walks through the ADE stage by stage. Accepting it from the wizard
 * adds that worker when it is not running yet — quietly, since it is how the
 * tour is delivered rather than a choice in setup — and waits for its page.
 */

import { getExtPage, whenExtPage } from '@/lib/ui-slots'
import { addWorkersWithProgress } from './api'
import { TOUR_PAGE, TOUR_WORKER } from './catalog'

/**
 * How long to wait for the page once the worker is connected: the console
 * registers it when it has loaded the worker's script. Past it the tour
 * opens anyway, and its pane fills in when the script arrives.
 */
export const TOUR_PAGE_TIMEOUT_MS = 15_000

/** Make the tour's page available to open. Throws a readable error. */
export async function prepareTour(): Promise<void> {
  if (getExtPage(TOUR_PAGE)) return
  await addWorkersWithProgress([TOUR_WORKER], () => undefined)
  await whenExtPage(TOUR_PAGE, TOUR_PAGE_TIMEOUT_MS)
}
