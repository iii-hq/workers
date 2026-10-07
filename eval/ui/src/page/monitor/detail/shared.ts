import type { EntryRef } from '../../../types'

/** What the masthead and the state notice can do for the open analysis. */
export interface DetailActions {
  busy: 'cancel' | 'reanalyze' | 'delete' | null
  cancel(): void
  reanalyze(): void
  remove(): void
  /** Absent when the console cannot open a conversation. */
  openSession?: () => void
  openInvestigation?: () => void
  viewSignals(): void
}

/** A chip was clicked: where its entry lives. `n` retriggers the same entry. */
export interface JumpTarget {
  entry: EntryRef
  n: number
}
