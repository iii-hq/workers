import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { TraceFilters } from '../components/TraceFilters'
import type { TraceFilterState } from '../hooks/useTraceFilters'

const INITIAL_FILTERS: TraceFilterState = {
  status: null,
  minDurationMs: null,
  maxDurationMs: null,
  startTime: null,
  endTime: null,
  sortBy: 'start_time',
  sortOrder: 'desc',
  groupBy: 'none',
  page: 1,
  pageSize: 50,
}

const STATS = { totalTraces: 4, errorCount: 1, avgDuration: 377 }

// TraceFilters is a controlled component: `filters` + `onFilterChange` and
// `searchQuery` + `onSearchChange` are value/onChange pairs that Storybook
// `args` can't drive on their own. This wrapper owns the state so the bar is
// actually interactive in the story. Only the passthrough props (stats,
// isLoading) come through as args.
interface HarnessProps {
  stats?: { totalTraces: number; errorCount: number; avgDuration: number }
  isLoading?: boolean
}

function Harness({ stats, isLoading }: HarnessProps) {
  const [filters, setFilters] = useState<TraceFilterState>(INITIAL_FILTERS)
  const [searchQuery, setSearchQuery] = useState('')

  return (
    <TraceFilters
      filters={filters}
      onFilterChange={(key, value) =>
        setFilters((prev) => ({ ...prev, [key]: value }))
      }
      onClear={() => setFilters(INITIAL_FILTERS)}
      searchQuery={searchQuery}
      onSearchChange={setSearchQuery}
      stats={stats}
      isLoading={isLoading}
    />
  )
}

const meta = {
  title: 'TracesV2/TraceFilters',
  component: Harness,
  parameters: { layout: 'centered' },
  decorators: [
    (Story) => (
      <div className="w-[900px] border border-rule bg-bg p-2">
        <Story />
      </div>
    ),
  ],
  args: { stats: STATS },
} satisfies Meta<typeof Harness>

export default meta
type Story = StoryObj<typeof meta>

export const Default: Story = {}

export const Loading: Story = { args: { isLoading: true } }

export const NoStats: Story = { args: { stats: undefined } }
