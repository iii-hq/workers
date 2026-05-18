import { ModeToggle } from '@/components/ui/ModeToggle'

export type ViewType = 'waterfall' | 'flamegraph' | 'map' | 'flow'

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
        { value: 'waterfall', label: 'waterfall' },
        { value: 'flamegraph', label: 'flame' },
        { value: 'map', label: 'map' },
        { value: 'flow', label: 'flow' },
      ]}
    />
  )
}
