import { useEffect } from 'react'
import { useConfigurationsList } from './hooks'
import { WorkerEditor, WorkerEditorEmptySelection } from './WorkerEditor'
import { WorkersList } from './WorkersList'

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
 * editor for the currently-selected id (URL-driven, see `useConfigurationRoute`).
 *
 * When the operator lands on `#/configuration/workers` with no id, we
 * auto-select the first entry once the list resolves so the editor surface
 * isn't blank by default. The auto-select is a one-shot — clearing the
 * selection later (e.g. by editing the hash) is preserved.
 *
 * Dirty-state propagation: `onDirtyChange` flows the editor's dirty
 * flag up to the Configuration shell, which uses it to gate tab + URL
 * navigation. The editor itself owns its draft state and only emits
 * dirty/clean transitions, not the draft value.
 */
export function WorkersTab({
  selectedId,
  onSelect,
  onDirtyChange,
}: WorkersTabProps) {
  const listQuery = useConfigurationsList()
  const entries = listQuery.data ?? []

  // Land on the first entry once the list resolves so the editor isn't
  // empty on the first visit. Subsequent renders only re-run when the
  // selection or the available ids change, which keeps a deliberate
  // `null` selection sticky after the operator clears it.
  useEffect(() => {
    if (selectedId) return
    if (entries.length === 0) return
    onSelect(entries[0].id)
  }, [selectedId, entries, onSelect])

  const selectedEntry =
    selectedId != null
      ? (entries.find((e) => e.id === selectedId) ?? null)
      : null

  return (
    <div className="workers-tab flex-1 flex min-h-0">
      <WorkersList
        configurations={entries}
        selectedId={selectedId}
        onSelect={onSelect}
        isLoading={listQuery.isLoading}
        isError={listQuery.isError}
        errorMessage={(listQuery.error as Error | null)?.message}
      />
      {selectedEntry ? (
        <WorkerEditor
          key={selectedEntry.id}
          entry={selectedEntry}
          onDirtyChange={onDirtyChange}
        />
      ) : (
        <WorkerEditorEmptySelection hasEntries={entries.length > 0} />
      )}
    </div>
  )
}
