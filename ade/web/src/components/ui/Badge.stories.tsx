import type { Meta, StoryObj } from '@storybook/react-vite'
import { Check, CircleAlert, Clock3, Radio, Sparkles } from 'lucide-react'
import { Badge } from './Badge'

const meta = {
  title: 'UI/Badge',
  component: Badge,
  args: { children: 'Status' },
  parameters: { layout: 'centered' },
} satisfies Meta<typeof Badge>

export default meta
type Story = StoryObj<typeof meta>

export const Variants: Story = {
  render: () => (
    <div className="flex flex-wrap items-center gap-2">
      <Badge>
        <Clock3 className="size-4" /> Neutral
      </Badge>
      <Badge variant="ok">
        <Check className="size-4" /> Active
      </Badge>
      <Badge variant="accent">
        <Sparkles className="size-4" /> New
      </Badge>
      <Badge variant="warn">
        <Radio className="size-4" /> Paused
      </Badge>
      <Badge variant="alert">
        <CircleAlert className="size-4" /> Failed
      </Badge>
    </div>
  ),
}
