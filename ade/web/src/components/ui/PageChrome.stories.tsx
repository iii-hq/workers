import type { Meta, StoryObj } from '@storybook/react-vite'
import { Files, RefreshCw } from 'lucide-react'
import { fn } from 'storybook/test'
import { Button } from './Button'
import { IconButton } from './IconButton'
import { Input } from './Input'
import { PageBody, PageHeader, PageMain, PageShell } from './PageChrome'

/* PageSidebar.stories covers the sidebar composition; this file is the
   header and the sidebar-less shell. */
const meta = {
  title: 'UI/PageChrome',
  component: PageHeader,
  parameters: { layout: 'fullscreen' },
  args: { onClose: fn() },
  render: (args) => (
    <div className="flex h-[280px]">
      <PageShell>
        <PageHeader {...args} />
        <PageBody>
          <PageMain className="items-center justify-center font-sans text-[12px] text-ink-faint">
            PageMain
          </PageMain>
        </PageBody>
      </PageShell>
    </div>
  ),
} satisfies Meta<typeof PageHeader>

export default meta
type Story = StoryObj<typeof meta>

/** Icon, title, description, actions and the pane's close affordance. */
export const Header: Story = {
  args: {
    icon: <Files />,
    title: 'Files',
    description: '~/workspaces/iii/workers',
    actions: (
      <>
        <IconButton label="refresh">
          <RefreshCw aria-hidden className="size-4" />
        </IconButton>
        <Button variant="pill" size="sm">
          New file
        </Button>
      </>
    ),
  },
}

export const TitleOnly: Story = { args: { title: 'Traces' } }

/** Children fill the flexible gap between the title and the actions. */
export const WithChildren: Story = {
  args: {
    icon: <Files />,
    title: 'Files',
    children: (
      <Input
        value=""
        onChange={fn()}
        placeholder="Filter files…"
        aria-label="Filter files"
        className="max-w-xs"
      />
    ),
  },
}

/** The description truncates first; title and actions keep their width. */
export const Narrow: Story = {
  args: {
    ...Header.args,
    description:
      '~/workspaces/iii/workers/ade/web/src/components/ui/PageChrome.tsx',
  },
  decorators: [
    (Story) => (
      <div className="w-[420px]">
        <Story />
      </div>
    ),
  ],
}
