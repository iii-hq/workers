import { Dialog, DialogContent, DialogTitle } from '@/components/ui/Dialog'
import { Skeleton } from '@/components/ui/Skeleton'
import { cn } from '@/lib/utils'
import { useConfigurationSchema } from '@/pages/Configuration/tabs/WorkersTab/hooks'
import { wt } from '@/pages/Configuration/tabs/WorkersTab/typography'
import { useUnsavedGuard } from '@/pages/Configuration/tabs/WorkersTab/useUnsavedGuard'
import { WorkerEditor } from '@/pages/Configuration/tabs/WorkersTab/WorkerEditor'

interface WorkerConfigurationDialogProps {
  configurationId: string | null
  onClose: () => void
}

export function WorkerConfigurationDialog({
  configurationId,
  onClose,
}: WorkerConfigurationDialogProps) {
  const guard = useUnsavedGuard()
  const open = configurationId !== null

  function handleOpenChange(nextOpen: boolean) {
    if (nextOpen) return
    guard.tryNavigate(onClose)
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent
        className={cn(
          'configuration-surface workers-tab flex h-[min(90vh,900px)] max-h-[90vh]',
          'w-[min(calc(100vw-2rem),1180px)] max-w-none flex-col overflow-hidden p-0 font-sans',
        )}
      >
        <DialogTitle className="sr-only">worker configuration</DialogTitle>
        {configurationId ? (
          <WorkerConfigurationBody
            configurationId={configurationId}
            onDirtyChange={guard.setDirty}
          />
        ) : null}
      </DialogContent>
    </Dialog>
  )
}

function WorkerConfigurationBody({
  configurationId,
  onDirtyChange,
}: {
  configurationId: string
  onDirtyChange: (dirty: boolean) => void
}) {
  const schemaQuery = useConfigurationSchema(configurationId)

  if (schemaQuery.isLoading) {
    return (
      <div className="flex-1 min-h-0 px-6 py-8 space-y-4">
        <Skeleton className="h-5 w-48" />
        <Skeleton className="h-10 w-full" />
        <Skeleton className="h-10 w-full" />
        <Skeleton className="h-5 w-32 mt-6" />
        <Skeleton className="h-10 w-full" />
      </div>
    )
  }

  if (schemaQuery.isError || !schemaQuery.data) {
    return (
      <div className="flex-1 min-h-0 px-6 py-8">
        <h2 className={cn(wt.heading, 'text-ink')}>configuration</h2>
        <p className={cn(wt.bodySm, 'mt-2 text-alert')} role="alert">
          {(schemaQuery.error as Error | null)?.message ??
            `failed to load ${configurationId}`}
        </p>
      </div>
    )
  }

  return (
    <WorkerEditor
      key={schemaQuery.data.id}
      entry={schemaQuery.data}
      onDirtyChange={onDirtyChange}
    />
  )
}
