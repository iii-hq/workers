import type { Meta, StoryObj } from '@storybook/react-vite'
import { Filter, RefreshCw } from 'lucide-react'
import { Button } from './Button'
import { IconButton } from './IconButton'
import { StatusBar, Toolbar } from './Toolbar'

const meta = {
  title: 'UI/Toolbar',
  component: Toolbar,
  parameters: { layout: 'padded' },
} satisfies Meta<typeof Toolbar>

export default meta
type Story = StoryObj<typeof meta>

/** Secondary strip under a page header: controls left, actions pushed right. */
export const Secondary: Story = {
  args: { 'aria-label': 'Sessions' },
  render: () => (
    <div className="w-[36rem] overflow-hidden rounded-sm border border-edge bg-panel">
      <Toolbar
        aria-label="Sessions"
        end={
          <IconButton label="Refresh">
            <RefreshCw className="size-4" aria-hidden />
          </IconButton>
        }
      >
        <Button variant="pill" size="sm" type="button">
          All
        </Button>
        <Button variant="pill" size="sm" type="button">
          <Filter className="size-4" aria-hidden />
          Filter
        </Button>
      </Toolbar>
      <div className="h-24" />
      <StatusBar end={<span>updated 4s ago</span>}>
        <span>12 sessions</span>
        <span aria-hidden>·</span>
        <span>3 live</span>
      </StatusBar>
    </div>
  ),
}
