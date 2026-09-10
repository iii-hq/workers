import type { Meta, StoryObj } from '@storybook/react-vite'
import { FileText, Plus } from 'lucide-react'
import { useCallback, useRef, useState } from 'react'
import { Button } from './Button'
import { IconButton } from './IconButton'
import { List, ListItem } from './List'
import { PageBody, PageMain, PageShell } from './PageChrome'
import { PageSidebar } from './PageSidebar'

const meta = {
  title: 'UI/PageSidebar',
  parameters: { layout: 'fullscreen' },
} satisfies Meta

export default meta
type Story = StoryObj

function Navigation({
  selected = 'Overview',
  onSelect,
}: {
  selected?: string | null
  onSelect?: (label: string) => void
}) {
  return (
    <List className="px-2 py-1">
      <ListItem
        selected={selected === 'Overview'}
        leading={<FileText />}
        label="Overview"
        onClick={() => onSelect?.('Overview')}
      />
      <ListItem
        leading={<FileText />}
        label="Configuration"
        selected={selected === 'Configuration'}
        onClick={() => onSelect?.('Configuration')}
      />
      <ListItem
        leading={<FileText />}
        label="Activity"
        selected={selected === 'Activity'}
        onClick={() => onSelect?.('Activity')}
      />
    </List>
  )
}

function ResponsiveNavigationExample() {
  const [narrow, setNarrow] = useState(false)
  const [selected, setSelected] = useState<string | null>(null)
  const observerRef = useRef<ResizeObserver | null>(null)
  const bodyRef = useCallback((node: HTMLDivElement | null) => {
    observerRef.current?.disconnect()
    observerRef.current = null
    if (!node) return

    const update = (width: number) => {
      if (width > 0) setNarrow(width <= 700)
    }
    update(node.getBoundingClientRect().width)
    const observer = new ResizeObserver(([entry]) => {
      if (entry) update(entry.contentRect.width)
    })
    observer.observe(node)
    observerRef.current = observer
  }, [])

  const showNavigation = !narrow || selected === null
  const showMain = !narrow || selected !== null

  return (
    <PageShell className="h-[28rem]">
      <div ref={bodyRef} className="flex min-h-0 min-w-0 flex-1">
        <PageBody>
          {showNavigation ? (
            <PageSidebar
              label="Navigation"
              defaultWidth={240}
              collapsible
              narrow={narrow}
            >
              <Navigation selected={selected} onSelect={setSelected} />
            </PageSidebar>
          ) : null}
          {showMain ? (
            <PageMain className="items-center justify-center gap-3 p-6 font-sans text-sm text-ink-faint">
              {narrow ? (
                <Button variant="ghost" onClick={() => setSelected(null)}>
                  Back to navigation
                </Button>
              ) : null}
              {selected ?? 'Select a navigation item'}
            </PageMain>
          ) : null}
        </PageBody>
      </div>
    </PageShell>
  )
}

function SidebarExample({
  side = 'left',
  narrowBelow,
  narrowMode,
}: {
  side?: 'left' | 'right'
  narrowBelow?: number
  narrowMode?: 'inline' | 'drawer'
}) {
  const newAction = (
    <IconButton
      label="new item"
      tooltipSide={side === 'left' ? 'right' : 'left'}
    >
      <Plus className="size-4" />
    </IconButton>
  )

  return (
    <PageShell className="h-[28rem]">
      <PageBody side={side}>
        <PageSidebar
          label="Navigation"
          side={side}
          defaultWidth={240}
          minWidth={180}
          maxWidth={360}
          collapsible
          resizable
          narrowBelow={narrowBelow}
          narrowMode={narrowMode}
          header={
            <div className="flex min-w-0 flex-1 items-center justify-between gap-2">
              <span className="truncate font-sans text-sm font-medium text-ink">
                Navigation
              </span>
              {newAction}
            </div>
          }
          collapsedActions={newAction}
        >
          <Navigation />
        </PageSidebar>
        <PageMain className="items-center justify-center p-6 font-sans text-sm text-ink-faint">
          {narrowBelow === undefined
            ? 'Resize the separator, then collapse and expand the shared sidebar.'
            : narrowMode === 'drawer'
              ? 'Below 700px the sidebar is a rail; expand it to open the drawer over this column.'
              : 'Below 700px navigation owns the full pane; selecting a row should advance the page drill-in.'}
        </PageMain>
      </PageBody>
    </PageShell>
  )
}

export const Left: Story = {
  render: () => <SidebarExample />,
}

export const Right: Story = {
  render: () => <SidebarExample side="right" />,
}

export const ResponsiveNavigation: Story = {
  render: () => <ResponsiveNavigationExample />,
}

export const ResponsiveDrawer: Story = {
  render: () => <SidebarExample narrowBelow={700} narrowMode="drawer" />,
}
