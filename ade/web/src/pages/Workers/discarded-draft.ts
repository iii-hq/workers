import type { ContainerEntry } from './compose-api'
import {
  rebaseSettingsDraft,
  type SettingsDraft,
  submittedSettingsEntry,
} from './model'

export type DiscardedSettingsDraft = {
  entry: ContainerEntry
  draft: SettingsDraft
}

/** Keeps a detached pane-memory draft; stored credentials remain write-only. */
export function captureDiscardedDraft(
  entry: ContainerEntry,
  draft: SettingsDraft,
): DiscardedSettingsDraft {
  const safeEntry = submittedSettingsEntry(entry, {})
  return {
    entry: safeEntry,
    draft: rebaseSettingsDraft(entry, draft, safeEntry),
  }
}

/** Stages discarded local edits against the latest baseline without applying them. */
export function restoreDiscardedDraft(
  snapshot: DiscardedSettingsDraft,
  latestEntry: ContainerEntry,
): SettingsDraft {
  return rebaseSettingsDraft(
    snapshot.entry,
    snapshot.draft,
    submittedSettingsEntry(latestEntry, {}),
  )
}
