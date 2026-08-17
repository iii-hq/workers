import { SlidersHorizontal } from 'lucide-react'
import { PageHeader, PageShell } from '@/components/ui/PageChrome'
import { WorkersTab } from '@/pages/Configuration/tabs/WorkersTab'
import { useUnsavedGuard } from '@/pages/Configuration/tabs/WorkersTab/useUnsavedGuard'

interface WorkerConfigurationPanelProps {
  selectedId: string | null
  /** Move within the surface: to an entry, or (null) back to the list. */
  onSelect: (configurationId: string | null) => void
  /** The header ✕ — leave the surface, back to the workers table. */
  onClose: () => void
}

/**
 * Full-pane worker configuration surface (replaces the old modal): the
 * standard page chrome over the `WorkersTab` master-detail, which brings
 * its own narrow drill-in flow. Selection is hash-routed by the caller so
 * deep links (`#/workers/configuration/<id>[/field…]`) keep working.
 *
 * The unsaved guard covers the exits this panel owns — the ✕ and browser
 * close/reload. In-surface navigation (row switches, drill-out) is
 * guarded inside `WorkersTab` itself.
 */
export function WorkerConfigurationPanel({
  selectedId,
  onSelect,
  onClose,
}: WorkerConfigurationPanelProps) {
  const guard = useUnsavedGuard()
  return (
    <PageShell
      className="configuration-surface font-sans"
      aria-label="worker configuration"
    >
      <PageHeader
        icon={<SlidersHorizontal />}
        title="worker configuration"
        description="schemas & values registered by workers"
        onClose={() => guard.tryNavigate(onClose)}
      />
      <WorkersTab
        selectedId={selectedId}
        onSelect={onSelect}
        onDirtyChange={guard.setDirty}
      />
    </PageShell>
  )
}
