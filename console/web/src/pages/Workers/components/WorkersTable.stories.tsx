import type { Meta, StoryObj } from '@storybook/react-vite'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { useMemo, useState } from 'react'
import { TooltipProvider } from '@/components/ui/Tooltip'
import {
  WORKER_SURFACE_FIXTURE,
  WORKERS_FIXTURE_EMPTY,
  WORKERS_FIXTURE_ROWS,
} from '../fixtures/workers-fixtures'
import type { WorkerRow } from '../types'
import { filterWorkerRows, type WorkersFilterState } from '../types'
import { workerSurfaceKeys } from './WorkerSurface'
import { WorkersFilters } from './WorkersFilters'
import { WorkersTable } from './WorkersTable'

/**
 * Expanding a row fetches `engine::workers::info` through react-query, so the
 * stories seed that cache instead of reaching for an engine. Seeding also
 * keeps the expanded row deterministic in the gallery.
 */
function storyQueryClient(): QueryClient {
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, staleTime: Number.POSITIVE_INFINITY },
    },
  })
  client.setQueryData(
    workerSurfaceKeys.detail('harness'),
    WORKER_SURFACE_FIXTURE,
  )
  return client
}

function WorkersHarness({
  rows,
  isLoading,
  initialTag,
  onStop,
  onConfigure,
}: {
  rows: WorkerRow[]
  isLoading?: boolean
  initialTag?: string | null
  onStop?: (name: string) => void
  onConfigure?: (configurationId: string) => void
}) {
  const [filters, setFilters] = useState<WorkersFilterState>({
    search: '',
    tag: initialTag ?? null,
    runtime: null,
  })
  const filtered = useMemo(
    () => filterWorkerRows(rows, filters),
    [rows, filters],
  )

  const [queryClient] = useState(storyQueryClient)

  return (
    <QueryClientProvider client={queryClient}>
      <TooltipProvider>
        <div className="flex flex-col gap-4 p-6 bg-bg min-h-[420px]">
          {!isLoading && rows.length > 0 ? (
            <WorkersFilters
              rows={rows}
              filters={filters}
              onFilterChange={(next) =>
                setFilters((cur) => ({ ...cur, ...next }))
              }
              onClear={() =>
                setFilters({ search: '', tag: null, runtime: null })
              }
            />
          ) : null}
          <WorkersTable
            rows={filtered}
            isLoading={isLoading}
            onStop={onStop}
            onConfigure={onConfigure}
          />
        </div>
      </TooltipProvider>
    </QueryClientProvider>
  )
}

const meta = {
  title: 'pages/Workers/WorkersTable',
  component: WorkersHarness,
  parameters: { layout: 'fullscreen' },
} satisfies Meta<typeof WorkersHarness>

export default meta
type Story = StoryObj<typeof meta>

export const Default: Story = {
  args: {
    rows: WORKERS_FIXTURE_ROWS,
    onConfigure: () => undefined,
  },
}

export const FilteredByTag: Story = {
  args: {
    rows: WORKERS_FIXTURE_ROWS,
    initialTag: 'platform',
    onConfigure: () => undefined,
  },
}

export const Empty: Story = {
  args: {
    rows: WORKERS_FIXTURE_EMPTY,
  },
}

export const Loading: Story = {
  args: {
    rows: [],
    isLoading: true,
  },
}

const supervisorRows = WORKERS_FIXTURE_ROWS.filter(
  (r) => r.managementKind === 'supervisor' && r.stopEnabled,
)

export const StopEnabled: Story = {
  args: {
    rows: supervisorRows,
    onStop: () => undefined,
    onConfigure: () => undefined,
  },
}

export const StandaloneNoStop: Story = {
  args: {
    rows: WORKERS_FIXTURE_ROWS.filter((r) => r.managementKind === 'standalone'),
  },
}

/**
 * A connected row expanded into its surface: the functions it registered,
 * the trigger types it publishes, and the bindings pointing into it. Click
 * the `harness` row in the canvas to open it.
 */
export const ExpandedSurface: Story = {
  args: {
    rows: WORKERS_FIXTURE_ROWS.filter((r) => r.name === 'harness'),
    onConfigure: () => undefined,
  },
  play: async ({ canvasElement }) => {
    const toggle = canvasElement.querySelector<HTMLButtonElement>(
      'button[aria-expanded="false"]',
    )
    toggle?.click()
  },
}
