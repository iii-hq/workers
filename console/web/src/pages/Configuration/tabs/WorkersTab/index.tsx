import { useCallback, useEffect, useState } from 'react'
import { useContainerNarrow } from '@/hooks/use-container-narrow'
import { useConfigurationsList, useWorkerRegistryReactivity } from './hooks'
import { WorkerEditor, WorkerEditorEmptySelection } from './WorkerEditor'
import { WorkersList } from './WorkersList'

/**
 * Container width (px) below which the tab collapses to the drill-in
 * list ⇄ editor flow (same pattern as the directory worker's page).
 */
const NARROW_BELOW = 720

interface WorkersTabProps {
  selectedId: string | null
  onSelect: (workerId: string | null) => void
  /**
   * Called whenever the editor's dirty state changes. Lifted up to the
   * Configuration page so the tab strip can intercept tab switches with
   * the same unsaved-changes confirm we use for worker selection.
   */
  onDirtyChange: (dirty: boolean) => void
}

/**
 * Master-detail layout for worker configurations. The left rail lists
 * every entry from `configuration::list`; the right pane shows the
 * editor for the currently-selected id.
 *
 * Layout adapts to the width the tab HAS (a ResizeObserver on its own
 * root, not a viewport media query — the console can host it in panes
 * of any size). Under NARROW_BELOW px it becomes a drill-in flow: the
 * list fills the width, opening a worker swaps the list for the editor
 * with a ← back button.
 *
 * When mounted wide with no id, we auto-select the first entry once the
 * list resolves so the editor surface isn't blank by default. In the
 * narrow flow the list IS the landing page, so no auto-select happens —
 * drilling straight past it would strand the back button on first paint.
 *
 * Dirty-state propagation: `onDirtyChange` flows the editor's dirty
 * flag up to the Configuration shell, which uses it to gate tab + URL
 * navigation. A local mirror of the same flag guards in-tab navigation
 * (row switches, drill-out) with a confirm. The editor itself owns its
 * draft state and only emits dirty/clean transitions, not the draft
 * value.
 */
export function WorkersTab({
  selectedId,
  onSelect,
  onDirtyChange,
}: WorkersTabProps) {
  const listQuery = useConfigurationsList()
  // Tombstone: the `coder` worker was folded into `shell`. Its configuration
  // entry is kept (inert) as a one-shot migration/rollback artifact, but it is
  // no longer a live worker, so hide it from the editor — surfacing it would
  // read as a second, editable filesystem config.
  const entries = (listQuery.data ?? []).filter((e) => e.id !== 'coder')

  // React to workers added/removed out of band (CLI `iii worker add/remove`)
  // by invalidating the list so a freshly-installed worker's config appears
  // without a manual reload.
  useWorkerRegistryReactivity()

  const [rootRef, narrow] = useContainerNarrow(NARROW_BELOW)

  // Local mirror of the editor's dirty flag so drill-out and row switches
  // can guard unsaved changes without a round-trip through the parent.
  const [dirty, setDirty] = useState(false)
  const handleDirtyChange = useCallback(
    (next: boolean) => {
      setDirty(next)
      onDirtyChange(next)
    },
    [onDirtyChange],
  )

  // Land on the first entry once the list resolves so the editor isn't
  // empty on the first visit — wide layouts only (see doc comment).
  // Subsequent renders only re-run when the selection or the available
  // ids change, which keeps a deliberate `null` selection sticky after
  // the operator clears it.
  useEffect(() => {
    if (narrow) return
    if (selectedId) return
    if (entries.length === 0) return
    onSelect(entries[0].id)
  }, [narrow, selectedId, entries, onSelect])

  const handleSelect = useCallback(
    (id: string | null) => {
      if (id === selectedId) return
      if (dirty && !window.confirm('discard unsaved changes?')) return
      onSelect(id)
    },
    [dirty, selectedId, onSelect],
  )

  const selectedEntry =
    selectedId != null
      ? (entries.find((e) => e.id === selectedId) ?? null)
      : null

  // Narrow: one pane at a time — the list, or the opened editor.
  const showList = !narrow || selectedId === null
  const showEditor = !narrow || selectedId !== null

  return (
    <div
      ref={rootRef}
      className="workers-tab flex-1 flex min-h-0 min-w-0 gap-px bg-edge"
    >
      {showList ? (
        <WorkersList
          configurations={entries}
          selectedId={selectedId}
          onSelect={handleSelect}
          isLoading={listQuery.isLoading}
          isError={listQuery.isError}
          errorMessage={(listQuery.error as Error | null)?.message}
          narrow={narrow}
        />
      ) : null}
      {showEditor ? (
        selectedEntry ? (
          <WorkerEditor
            key={selectedEntry.id}
            entry={selectedEntry}
            onDirtyChange={handleDirtyChange}
            onBack={narrow ? () => handleSelect(null) : undefined}
          />
        ) : (
          <WorkerEditorEmptySelection hasEntries={entries.length > 0} />
        )
      ) : null}
    </div>
  )
}
