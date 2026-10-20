/**
 * The learn-iii tour opens by itself once the operator has had the ADE for
 * `TOUR_AFTER_MS`: five minutes from the first time this ADE was opened
 * (`first_opened_at`, which the backend records on the first
 * `console::onboarding::get` and keeps across restarts), and once only
 * (`tour_opened_at`). A reload or a restart past the mark opens it at once;
 * one before it arms the rest of the wait. The command palette's
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

/** How long the operator has the ADE before the tour offers itself. */
export const TOUR_AFTER_MS = 5 * 60_000

/**
 * When the tour is due, from the state as read: `null` when it already
 * opened, when the ADE never recorded a first open (a backend older than the
 * gate), or when nothing may open by itself here.
 */
export function tourDueAt(
  state: OnboardingState | null,
  automated = browserIsAutomated(),
): number | null {
  if (!state || automated || state.auto_open === false) return null
  if (state.tour_opened_at || typeof state.first_opened_at !== 'number') {
    return null
  }
  return state.first_opened_at + TOUR_AFTER_MS
}

/**
 * Arm the gate. Mounted once beside the wizard host, which says whether the
 * wizard is up; the tour waits for it to close. `live` is a real backend.
 */
export function useTourGate(wizardOpen: boolean, live: boolean): void {
  useEffect(() => {
    if (!live || wizardOpen) return
    let active = true
    let timer: number | undefined
    const fire = () => {
      if (!active) return
      void prepareTour()
        .then(openTour)
        .then(markTourOpened)
        .catch((error: unknown) => {
          // Nobody asked for the tour in the moment, so a failure is not
          // worth a dialog; the palette's `Open learn-iii` still works.
          console.warn('[learn-iii] the tour could not open', error)
        })
    }
    void fetchOnboardingState()
      .then((state) => {
        if (!active) return
        const dueAt = tourDueAt(state)
        if (dueAt === null) return
        timer = window.setTimeout(fire, Math.max(0, dueAt - Date.now()))
      })
      .catch(() => undefined)
    return () => {
      active = false
      window.clearTimeout(timer)
    }
  }, [wizardOpen, live])
}
