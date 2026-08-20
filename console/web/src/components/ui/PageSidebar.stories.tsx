import type { Meta, StoryObj } from '@storybook/react-vite'
import { FileText, Plus } from 'lucide-react'
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

function Navigation() {
  return (
    <List className="px-2 py-1">
      <ListItem selected leading={<FileText />} label="Overview" />
      <ListItem leading={<FileText />} label="Configuration" />
      <ListItem leading={<FileText />} label="Activity" />
    </List>
  )
}

function SidebarExample({
  side = 'left',
  narrowBelow,
}: {
  side?: 'left' | 'right'
  narrowBelow?: number
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
        <PageMain
          className={`items-center justify-center p-6 font-sans text-sm text-ink-faint${
            narrowBelow === undefined ? '' : ' max-[700px]:hidden'
          }`}
        >
          Resize the separator, then collapse and expand the shared sidebar.
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

export const Responsive: Story = {
  render: () => <SidebarExample narrowBelow={700} />,
}
