import type { Meta, StoryObj } from '@storybook/react-vite'
import { Info } from 'lucide-react'
import { IconButton } from './IconButton'
import { Tooltip, TooltipContent, TooltipTrigger } from './Tooltip'

const meta = {
  title: 'UI/Tooltip',
  parameters: { layout: 'centered' },
} satisfies Meta

export default meta
type Story = StoryObj

export const SharedIconButton: Story = {
  render: () => (
    <IconButton label="inspect worker details">
      <Info className="size-4" aria-hidden />
    </IconButton>
  ),
}

export const LongContentNearEdge: Story = {
  render: () => (
    <div className="flex w-[240px] justify-end">
      <Tooltip defaultOpen>
        <TooltipTrigger asChild>
          <button
            type="button"
            className="rounded-sm bg-surface px-3 py-2 font-mono text-[13px] text-ink"
          >
            constrained trigger
          </button>
        </TooltipTrigger>
        <TooltipContent>
          long tooltip content wraps and collision-detects instead of escaping a
          narrow Console pane at 200% zoom
        </TooltipContent>
      </Tooltip>
    </div>
  ),
}
