/**
 * The guided tour that follows setup lives in the `learn-iii` worker. Its
 * UI floats over the workspace as an overlay (a page on consoles without
 * the overlay slot), and the worker's `Open learn-iii` command is what
 * shows it either way. Accepting the tour from the wizard adds that worker
 * when it is not running yet, and waits for its command. Adding it changes
 * the project (its worker-compose.yaml gains the worker and whatever it
 * needs), so the wizard says so while it happens: `prepareTour` reports
 * each move.
 */

import { getPageCommands, subscribePageCommands } from '@/lib/page-commands'
import { requestPanelOpen } from '@/lib/panel-context'
import { getExtPage, whenExtPage } from '@/lib/ui-slots'
import { addWorkersWithProgress, type StepProgress } from './api'
import { TOUR_PAGE, TOUR_WORKER } from './catalog'

/**
 * How long to wait for the tour once the worker is connected: the console
 * registers its command when it has loaded the worker's script. Past it the
 * tour opens anyway, and its pane fills in when the script arrives.
 */
export const TOUR_PAGE_TIMEOUT_MS = 15_000

/** The worker's `Open learn-iii` palette row, `<page>.<command>`. */
const TOUR_COMMAND_KEY = `${TOUR_PAGE}.open`

function tourCommand() {
  return getPageCommands().find(
    (entry) => entry.key === TOUR_COMMAND_KEY && entry.source === 'worker',
  )
}

/** Resolves once the tour's command is registered, or `false` at the deadline. */
function whenTourCommand(timeoutMs: number): Promise<boolean> {
  if (tourCommand()) return Promise.resolve(true)
  return new Promise((resolve) => {
    const finish = (found: boolean) => {
      clearTimeout(timer)
      unsubscribe()
      resolve(found)
    }
    const unsubscribe = subscribePageCommands(() => {
      if (tourCommand()) finish(true)
    })
    const timer = setTimeout(() => finish(false), timeoutMs)
  })
}

/** What getting the tour ready is doing, for the wizard to say. */
export interface TourProgress {
  /** The registry worker being added; absent once it only loads. */
  adding?: string
  /** The add's latest step, as Compose reports it. */
  note?: string
}

/** Make the tour available to open. Throws a readable error. */
export async function prepareTour(
  onProgress: (progress: TourProgress) => void = () => undefined,
): Promise<void> {
  if (tourCommand() || getExtPage(TOUR_PAGE)) return
  onProgress({ adding: TOUR_WORKER })
  await addWorkersWithProgress([TOUR_WORKER], (progress: StepProgress) =>
    onProgress({ adding: TOUR_WORKER, note: progress.note }),
  )
  onProgress({})
  // A worker older than the overlay registers its page and the command
  // together; either one landing means the script has loaded.
  await Promise.race([
    whenTourCommand(TOUR_PAGE_TIMEOUT_MS),
    whenExtPage(TOUR_PAGE, TOUR_PAGE_TIMEOUT_MS),
  ])
}

/**
 * Show the tour: the worker's own command when it has loaded (the overlay,
 * or its page on an older console), else the page by id, which fills in
 * when the script arrives.
 */
export function openTour(): void {
  const command = tourCommand()
  if (command) command.command.run()
  else requestPanelOpen({ pageId: TOUR_PAGE })
}
