import { useSyncExternalStore } from 'react'

/**
 * Whether the tour card is up, folded to its pill, or away. Browser-local on
 * purpose: progress is the engine's (a reload in another browser resumes the
 * same step), but how much of the screen the card takes is this window's
 * business. Kept in storage so a reload leaves the card the way it was.
 */
export type TourView = 'open' | 'minimized' | 'closed'

const STORAGE_KEY = 'iii.onboarding.view'
const listeners = new Set<() => void>()

function stored(): TourView {
  try {
    const value = window.localStorage.getItem(STORAGE_KEY)
    if (value === 'open' || value === 'minimized') return value
  } catch {
    // Storage blocked: the card starts away, like a first visit.
  }
  return 'closed'
}

let view: TourView = stored()

function set(next: TourView): void {
  if (next === view) return
  view = next
  try {
    window.localStorage.setItem(STORAGE_KEY, next)
  } catch {
    // Storage blocked: the view lasts as long as the page.
  }
  for (const listener of [...listeners]) listener()
}

/** Bring the card up: the console's `Open onboarding`, the wizard, the pill. */
export const showTour = () => set('open')
/** Fold the card to its pill; the tour keeps its place. */
export const minimizeTour = () => set('minimized')
/** Put the card away until it is asked for again. */
export const closeTour = () => set('closed')

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

export function useTourView(): TourView {
  return useSyncExternalStore(
    subscribe,
    () => view,
    () => view,
  )
}
