/**
 * The learn-iii tour opens by itself as soon as setup ends
 * (`setup_ended_at`, which the backend records when the wizard finishes or
 * is dismissed and keeps across restarts; `completed_at` stands in for a
 * state written before that field), and once only (`tour_opened_at`). A
 * reload or a restart before it opened opens it then. The command palette's
 * `Open learn-iii` is how it is shown again.
 *
 * Never over the setup wizard (the gate waits for it to close), never where
 * the wizard may not open by itself either (`auto_open: false`, a deployed
 * ADE), and never in a browser under automation.
 */

import { useEffect } from 'react'
import {
  fetchOnboardingState,
  markTourOpened,
  type OnboardingState,
} from './api'
import { browserIsAutomated } from './open'
import { openTour, prepareTour } from './tour'

/**
 * The tour is due: setup has ended, it has not opened yet, and something may
 * open by itself here.
 */
export function tourIsDue(
  state: OnboardingState | null,
  automated = browserIsAutomated(),
): boolean {
  if (!state || automated || state.auto_open === false) return false
  const endedAt = state.setup_ended_at ?? state.completed_at
  return !state.tour_opened_at && typeof endedAt === 'number'
}

/**
 * Arm the gate. Mounted once beside the wizard host, which says whether the
 * wizard is up; the tour waits for it to close. `live` is a real backend.
 */
export function useTourGate(wizardOpen: boolean, live: boolean): void {
  useEffect(() => {
    if (!live || wizardOpen) return
    let active = true
    void fetchOnboardingState()
      .then((state) => {
        if (!active || !tourIsDue(state)) return
        return prepareTour().then(openTour).then(markTourOpened)
      })
      .catch((error: unknown) => {
        // Nobody asked for the tour in the moment, so a failure is not
        // worth a dialog; the palette's `Open learn-iii` still works.
        console.warn('[learn-iii] the tour could not open', error)
      })
    return () => {
      active = false
    }
  }, [wizardOpen, live])
}
