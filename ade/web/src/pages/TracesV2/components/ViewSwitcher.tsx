import { GitBranch, Rows3 } from 'lucide-react'
import { Tabs, TabsList, TabsTrigger } from '@/components/shadcn/tabs'

export type ViewType = 'timeline' | 'waterfall'

interface ViewSwitcherProps {
  currentView: ViewType
  onViewChange: (next: ViewType) => void
}

/** Timeline or waterfall, as a segmented pair under the trace's stats. */
export function ViewSwitcher({ currentView, onViewChange }: ViewSwitcherProps) {
  return (
    <Tabs
      value={currentView}
      onValueChange={(next) => onViewChange(next as ViewType)}
    >
      <TabsList>
        <TabsTrigger value="timeline">
          <GitBranch data-icon="inline-start" />
          Timeline
        </TabsTrigger>
        <TabsTrigger value="waterfall">
          <Rows3 data-icon="inline-start" />
          Waterfall
        </TabsTrigger>
      </TabsList>
    </Tabs>
  )
}
