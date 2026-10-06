import { RefreshCw } from 'lucide-react'
import { useCallback, useRef } from 'react'
import { Button } from '@/components/ui/Button'
import { Skeleton } from '@/components/ui/Skeleton'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { cn } from '@/lib/utils'
import type { JsonValue } from '@/pages/Configuration/tabs/WorkersTab/api'
import { useConfigurationSchema } from '@/pages/Configuration/tabs/WorkersTab/hooks'
import { WorkerEditor } from '@/pages/Configuration/tabs/WorkersTab/WorkerEditor'

export interface WorkerConfigurationPanelProps {
  /** The configuration entry to edit; `null` renders nothing. */
  configurationId: string | null
  /** Unsaved edits appeared or went away: guard leaving the panel. */
  onDirtyChange?: (dirty: boolean) => void
  /** A save landed; receives the stored value. */
  onSaved?: (value: JsonValue) => void
  className?: string
}

/**
 * One worker's settings where the choice is made — a picker page, a sheet —
 * instead of in the Settings modal: the same editor (the worker's registered
 * form, schema validation, the save bar), without the Settings chrome. The
 * surface around it gives the title and the way back.
 */
export function WorkerConfigurationPanel({
  configurationId,
  onDirtyChange,
  onSaved,
  className,
}: WorkerConfigurationPanelProps) {
  const schemaQuery = useConfigurationSchema(configurationId)
  // The editor re-runs its dirty effects when these change identity; a
  // worker page passing inline arrows must not churn them every render.
  const dirtyRef = useRef(onDirtyChange)
  dirtyRef.current = onDirtyChange
  const savedRef = useRef(onSaved)
  savedRef.current = onSaved
  const reportDirty = useCallback((dirty: boolean) => {
    dirtyRef.current?.(dirty)
  }, [])
  const reportSaved = useCallback((value: JsonValue) => {
    savedRef.current?.(value)
  }, [])

  if (!configurationId) return null
  const entry =
    schemaQuery.data?.id === configurationId ? schemaQuery.data : null

  return (
    <div
      className={cn(
        'configuration-surface flex min-h-0 flex-1 flex-col overflow-hidden',
        className,
      )}
    >
      {schemaQuery.error ? (
        <div className="px-4 pb-4">
          <StatusPanel
            role="alert"
            variant="alert"
            headline="Could not load these settings"
            detail={
              schemaQuery.error instanceof Error
                ? schemaQuery.error.message
                : 'The configuration worker did not answer.'
            }
            action={
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() => void schemaQuery.refetch()}
                className="shrink-0"
              >
                <RefreshCw className="size-4" aria-hidden />
                Retry
              </Button>
            }
          />
        </div>
      ) : entry ? (
        <WorkerEditor
          key={entry.id}
          entry={entry}
          embedded
          onDirtyChange={reportDirty}
          onSaved={reportSaved}
        />
      ) : (
        <div
          className="space-y-4 px-4 py-2"
          role="status"
          aria-label="Loading settings"
        >
          <Skeleton className="h-5 w-36" />
          <Skeleton className="h-10 w-full" />
          <Skeleton className="h-10 w-full" />
        </div>
      )}
    </div>
  )
}
