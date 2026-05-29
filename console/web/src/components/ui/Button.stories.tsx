import type { Meta, StoryObj } from '@storybook/react-vite'
import { fn } from 'storybook/test'
import { Button } from './Button'
import { Prompt } from './Prompt'

const meta = {
  title: 'UI/Button',
  component: Button,
  parameters: { layout: 'padded' },
  args: { onClick: fn(), children: 'button' },
  argTypes: {
    variant: {
      control: 'select',
      options: ['primary', 'ghost', 'pill', 'icon', 'terminal', 'wiggle'],
    },
    size: { control: 'select', options: ['sm', 'md', 'lg', 'icon'] },
  },
} satisfies Meta<typeof Button>

export default meta
type Story = StoryObj<typeof meta>

export const Primary: Story = {
  args: { variant: 'primary', children: 'primary' },
}
export const Ghost: Story = { args: { variant: 'ghost', children: 'ghost' } }
export const Pill: Story = {
  args: { variant: 'pill', size: 'sm', children: 'pill' },
}
export const Terminal: Story = {
  args: {
    variant: 'terminal',
    children: (
      <>
        <Prompt symbol="$" /> npm install
      </>
    ),
  },
}
export const Wiggle: Story = { args: { variant: 'wiggle', children: 'wiggle' } }
export const Disabled: Story = {
  args: { variant: 'primary', disabled: true, children: 'disabled' },
}

export const IconButton: Story = {
  name: 'icon',
  args: {
    variant: 'icon',
    size: 'icon',
    'aria-label': 'settings',
    children: (
      <svg
        width="14"
        height="14"
        viewBox="0 0 14 14"
        fill="none"
        stroke="currentColor"
        strokeWidth="1"
        aria-hidden="true"
      >
        <path d="M2 4H12M2 7H12M2 10H12" />
      </svg>
    ),
  },
}

export const Sizes: Story = {
  render: (args) => (
    <div className="flex flex-wrap gap-3 items-center">
      <Button {...args} size="sm">
        sm
      </Button>
      <Button {...args} size="md">
        md
      </Button>
      <Button {...args} size="lg">
        lg
      </Button>
    </div>
  ),
}
