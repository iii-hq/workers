import { ModeToggle } from '@/components/ui/ModeToggle'

// TracesV2 lab scope: the detail offers the lane timeline (default — the
// same visual grammar as the live strip up top) and the waterfall tree.
// The flame graph was replaced by the timeline; map/flow views live in the
// original Traces surface and are out of scope here.
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
