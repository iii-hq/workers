import { GitBranch, Rows3 } from 'lucide-react'
import { Tabs, TabsList, TabsTrigger } from '@/components/shadcn/tabs'

export type ViewType = 'timeline' | 'waterfall'

interface ViewSwitcherProps {
  currentView: ViewType
  onViewChange: (next: ViewType) => void
}

/** The chosen segment wears the console's inverse fill (ink on the page,
 *  page on the ink) — the same mark the icon toggles use — so the active
 *  view is unmistakable on both themes; the other segment stays quiet. */
const SEGMENT =
  'h-6 rounded-[5px] px-2 text-xs font-medium text-muted-foreground transition-[background-color,color] duration-[var(--motion-duration-fast)] has-data-[icon=inline-start]:pl-1.5 hover:text-foreground data-active:bg-foreground data-active:text-background data-active:shadow-none dark:data-active:border-transparent dark:data-active:bg-foreground dark:data-active:text-background'

/** Timeline or waterfall, as a compact segmented pair in the detail toolbar. */
export function ViewSwitcher({ currentView, onViewChange }: ViewSwitcherProps) {
  return (
    <Tabs
      value={currentView}
      onValueChange={(next) => onViewChange(next as ViewType)}
    >
      <TabsList
        className="h-7 rounded-md border border-border bg-muted/60 p-0.5"
        aria-label="trace view"
      >
        <TabsTrigger value="timeline" className={SEGMENT}>
          <GitBranch data-icon="inline-start" className="size-4" />
          Timeline
        </TabsTrigger>
        <TabsTrigger value="waterfall" className={SEGMENT}>
          <Rows3 data-icon="inline-start" className="size-4" />
          Waterfall
        </TabsTrigger>
      </TabsList>
    </Tabs>
  )
}
