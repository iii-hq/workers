import { AlertCircle, Blocks, RefreshCw } from 'lucide-react'
import { useEffect, useRef } from 'react'
import { Button } from '@/components/ui/Button'
import { PageHeader, PageShell } from '@/components/ui/PageChrome'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { useWorkersConfigurationRoute } from '@/hooks/use-hash-route'
import { cn } from '@/lib/utils'
import type { PageCommandsApi } from '@/types/injectable-ui'
import { WorkerConfigurationPanel } from './components/WorkerConfigurationPanel'
import { WorkersFilters } from './components/WorkersFilters'
import { WorkersTable } from './components/WorkersTable'
import { useWorkersLive } from './hooks/useWorkersLive'

interface WorkersProps {
  /** Close the hosting pane — the header's standard ✕ when present. */
  onRequestClose?: () => void
  /** The pane's command registrar: the page's verbs and keys. */
  commands?: PageCommandsApi
}

export function Workers({ onRequestClose, commands }: WorkersProps) {
  const [configurationRoute, navigateConfiguration, closeConfiguration] =
    useWorkersConfigurationRoute()
  const {
    rows,
    allRows,
    filters,
    updateFilters,
    clearFilters,
    isLoading,
    isError,
    error,
    refetch,
    stoppingName,
    stopWorker,
  } = useWorkersLive()
  const rootRef = useRef<HTMLDivElement>(null)
  useEffect(
    () =>
      commands?.register([
        {
          id: 'refresh',
          title: 'Refresh workers',
          detail: 'Read the fleet again',
          keywords: ['reload', 'fleet'],
          shortcut: 'R',
          run: () => void refetch(),
        },
        {
          id: 'search',
          title: 'Search workers',
          detail: 'Put the caret in the filter',
          keywords: ['filter', 'find'],
          shortcut: '/',
          run: () =>
            rootRef.current
              ?.querySelector<HTMLElement>('[aria-label="search workers"]')
              ?.focus(),
        },
      ]),
    [commands, refetch],
  )

  const countLabel = isLoading ? '…' : String(allRows.length)

  return (
    <TooltipProvider>
      {configurationRoute.open ? (
        <WorkerConfigurationPanel
          selectedId={configurationRoute.configurationId}
          onSelect={navigateConfiguration}
          onClose={closeConfiguration}
        />
      ) : (
        <PageShell aria-label="workers">
          <PageHeader
            icon={<Blocks />}
            title="Workers"
            description={`${countLabel} connected or installed`}
            onClose={onRequestClose}
            actions={
              <Button
                variant="ghost"
                size="sm"
                onClick={() => void refetch()}
                disabled={isLoading}
                className="gap-1.5"
              >
                <RefreshCw
                  className={cn('size-4', isLoading && 'animate-spin')}
                  aria-hidden
                />
                refresh
              </Button>
            }
          />
          <div className="flex-1 overflow-y-auto px-4 sm:px-6 lg:px-8 py-4 space-y-4">
            {isError ? (
              <StatusPanel
                variant="alert"
                icon={<AlertCircle className="w-full h-full" />}
                headline="failed to load workers"
                detail={
                  error instanceof Error
                    ? error.message
                    : 'check that the engine is running and reachable.'
                }
              />
            ) : null}

            <WorkersFilters
              rows={allRows}
              filters={filters}
              onFilterChange={updateFilters}
              onClear={clearFilters}
            />

            <WorkersTable
              rows={rows}
              isLoading={isLoading}
              stoppingName={stoppingName}
              onStop={stopWorker}
              onConfigure={(configurationId) =>
                navigateConfiguration(configurationId)
              }
            />
          </div>
        </PageShell>
      )}
    </TooltipProvider>
  )
}
