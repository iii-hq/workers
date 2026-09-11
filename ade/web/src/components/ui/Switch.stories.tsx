import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { Switch } from './Switch'

const meta = {
  title: 'UI/Switch',
  component: Switch,
  parameters: { layout: 'centered' },
  args: {
    'aria-label': 'System notifications',
    name: 'system-notifications',
  },
} satisfies Meta<typeof Switch>

export default meta
type Story = StoryObj<typeof meta>

export const Unchecked: Story = {}

export const Checked: Story = {
  args: { defaultChecked: true },
}

export const Disabled: Story = {
  args: { defaultChecked: true, disabled: true },
}

function ControlledPreview() {
  const [checked, setChecked] = useState(true)
  return (
    <Switch
      aria-label="Critical requests"
      name="critical-requests"
      checked={checked}
      onChange={(event) => setChecked(event.currentTarget.checked)}
    />
  )
}

export const Controlled: Story = {
  render: () => <ControlledPreview />,
}
