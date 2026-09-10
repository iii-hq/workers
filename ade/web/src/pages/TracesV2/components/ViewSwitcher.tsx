import { ModeToggle } from '@/components/ui/ModeToggle'

// The detail offers the lane timeline (default — the same visual grammar
// as the live strip up top) and the waterfall tree. The original Traces
// surface's flame graph was replaced by the timeline; its map/flow views
// were retired with it.
export type ViewType = 'timeline' | 'waterfall'

interface ViewSwitcherProps {
  currentView: ViewType
  onViewChange: (next: ViewType) => void
}

export function ViewSwitcher({ currentView, onViewChange }: ViewSwitcherProps) {
  return (
    <ModeToggle<ViewType>
      value={currentView}
      onChange={onViewChange}
      options={[
        { value: 'timeline', label: 'timeline' },
        { value: 'waterfall', label: 'waterfall' },
      ]}
    />
  )
}
