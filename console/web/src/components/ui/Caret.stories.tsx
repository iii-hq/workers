import type { Meta, StoryObj } from '@storybook/react-vite'
import { Caret } from './Caret'

const meta = {
  title: 'UI/Caret',
  component: Caret,
  parameters: { layout: 'padded' },
} satisfies Meta<typeof Caret>

export default meta
type Story = StoryObj<typeof meta>

export const Default: Story = {
  render: () => (
    <div className="font-mono text-[14px] text-ink leading-[1.7]">
      running engine::echo
      <Caret className="ml-1" />
    </div>
  ),
}
