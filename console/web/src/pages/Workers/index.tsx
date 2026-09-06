import { AlertCircle, Blocks, Layers, RefreshCw } from 'lucide-react'
import { useEffect, useRef } from 'react'
import { Button } from '@/components/ui/Button'
import { PageHeader, PageShell } from '@/components/ui/PageChrome'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { requestPanelOpen } from '@/lib/panel-context'
import { useExtPages } from '@/lib/ui-slots'
import { cn } from '@/lib/utils'
import type { PageCommandsApi } from '@/types/injectable-ui'
import { WorkersFilters } from './components/WorkersFilters'
import { WorkersTable } from './components/WorkersTable'
import { useWorkersLive } from './hooks/useWorkersLive'

const COMPOSE_PAGE_ID = 'compose'

interface WorkersProps {
  /** Close the hosting pane — the header's standard ✕ when present. */
  onRequestClose?: () => void
  /** The pane's command registrar: the page's verbs and keys. */
  commands?: PageCommandsApi
}

export function Workers({ onRequestClose, commands }: WorkersProps) {
  const {
    rows,
    allRows,
    compose,
    filters,
    updateFilters,
    clearFilters,
    isLoading,
    isError,
    error,
    refetch,
    stoppingName,
    stopWorker,
    pendingCompose,
    composeAction,
    composeError,
  } = useWorkersLive()
  const composePage = useExtPages().some((page) => page.id === COMPOSE_PAGE_ID)
  const rootRef = useRef<HTMLDivElement>(null)
  useEffect(
    () =>
      commands?.register([
        {
          id: 'refresh',
          title: 'Refresh workers',
          detail: 'Read the fleet again',
          keywords: ['reload', 'fleet'],
          run: () => void refetch(),
        },
        {
          id: 'search',
          title: 'Search workers',
          detail: 'Put the caret in the filter',
          keywords: ['filter', 'find'],
          run: () =>
            rootRef.current
              ?.querySelector<HTMLElement>('[aria-label="search workers"]')
              ?.focus(),
        },
        {
          id: 'compose',
          title: 'Open Compose',
          detail: 'Containers, lifecycle, worker packages, and logs',
          keywords: ['compose', 'containers', 'daemon', 'logs'],
          enabled: () => composePage,
          run: () => requestPanelOpen({ pageId: COMPOSE_PAGE_ID, context: {} }),
        },
      ]),
    [commands, refetch, composePage],
  )

  const countLabel = isLoading ? '…' : String(allRows.length)
  const description = compose
    ? `${countLabel} connected or declared · compose${compose.namespace ? ` ${compose.namespace}` : ''} ${compose.ready}/${compose.total} ready`
    : `${countLabel} connected or installed`

  return (
    <TooltipProvider>
      <PageShell ref={rootRef} aria-label="workers">
        <PageHeader
          icon={<Blocks />}
          title="Workers"
          description={description}
          onClose={onRequestClose}
          actions={
            <>
              {composePage ? (
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() =>
                    requestPanelOpen({ pageId: COMPOSE_PAGE_ID, context: {} })
                  }
                  className="gap-1.5"
                >
                  <Layers className="size-4" aria-hidden />
                  compose
                </Button>
              ) : null}
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
            </>
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

          {composeError ? (
            <StatusPanel
              variant="alert"
              icon={<AlertCircle className="w-full h-full" />}
              headline="compose action failed"
              detail={
                composeError instanceof Error
                  ? composeError.message
                  : String(composeError)
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
            pendingCompose={pendingCompose}
            onStop={stopWorker}
            onComposeAction={composeAction}
          />
        </div>
      </PageShell>
    </TooltipProvider>
  )
}
