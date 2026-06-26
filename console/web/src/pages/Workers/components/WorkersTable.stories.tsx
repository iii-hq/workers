import type { Meta, StoryObj } from '@storybook/react-vite'
import { useMemo, useState } from 'react'
import { TooltipProvider } from '@/components/ui/Tooltip'
import {
  WORKERS_FIXTURE_EMPTY,
  WORKERS_FIXTURE_ROWS,
} from '../fixtures/workers-fixtures'
import type { WorkerRow } from '../types'
import {
  filterWorkerRows,
  type WorkersFilterState,
} from '../types'
import { WorkersFilters } from './WorkersFilters'
import { WorkersTable } from './WorkersTable'

function WorkersHarness({
  rows,
  isLoading,
  initialTag,
  onStop,
}: {
  rows: WorkerRow[]
  isLoading?: boolean
  initialTag?: string | null
  onStop?: (name: string) => void
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

  return (
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
        />
      </div>
    </TooltipProvider>
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
  },
}

export const FilteredByTag: Story = {
  args: {
    rows: WORKERS_FIXTURE_ROWS,
    initialTag: 'platform',
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
  },
}

export const StandaloneNoStop: Story = {
  args: {
    rows: WORKERS_FIXTURE_ROWS.filter(
      (r) => r.managementKind === 'standalone',
    ),
  },
}
