import type { Meta, StoryObj } from '@storybook/react-vite'
import { Prompt } from './Prompt'

const meta = {
  title: 'UI/Prompt',
  component: Prompt,
  parameters: { layout: 'padded' },
} satisfies Meta<typeof Prompt>

export default meta
type Story = StoryObj<typeof meta>

export const WithText: Story = {
  args: { symbol: '$', children: 'install the latest release' },
}

export const SymbolOnly: Story = {
  args: { symbol: '>' },
}
