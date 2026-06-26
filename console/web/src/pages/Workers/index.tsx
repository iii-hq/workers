import { AlertCircle, RefreshCw } from 'lucide-react'
import { Button } from '@/components/ui/Button'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { cn } from '@/lib/utils'
import { WorkersFilters } from './components/WorkersFilters'
import { WorkersTable } from './components/WorkersTable'
import { useWorkersLive } from './hooks/useWorkersLive'

export function Workers() {
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

  const countLabel = isLoading
    ? '…'
    : String(allRows.length)

  return (
    <TooltipProvider>
      <main
        className="flex-1 flex flex-col min-h-0 overflow-hidden"
        aria-label="workers"
      >
        <header className="shrink-0 px-4 sm:px-6 lg:px-8 py-4 border-b border-rule flex flex-wrap items-center justify-between gap-3">
          <div>
            <h1 className="font-mono text-[16px] font-semibold tracking-[-0.01em] text-ink lowercase">
              workers
            </h1>
            <p className="font-mono text-[12px] text-ink-faint mt-0.5 lowercase">
              {countLabel} connected or installed
            </p>
          </div>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => void refetch()}
            disabled={isLoading}
            className="gap-1.5"
          >
            <RefreshCw
              className={cn('w-3.5 h-3.5', isLoading && 'animate-spin')}
              aria-hidden
            />
            refresh
          </Button>
        </header>

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
          />
        </div>
      </main>
    </TooltipProvider>
  )
}
