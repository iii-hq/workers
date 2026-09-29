import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { Checkbox } from './Checkbox'

const meta = {
  title: 'UI/Checkbox',
  component: Checkbox,
  parameters: { layout: 'centered' },
  args: { label: 'Include hidden files', name: 'hidden' },
} satisfies Meta<typeof Checkbox>

export default meta
type Story = StoryObj<typeof meta>

export const Unchecked: Story = {}

export const Checked: Story = {
  args: { defaultChecked: true },
}

export const Indeterminate: Story = {
  args: { indeterminate: true, label: 'Select all' },
}

export const Disabled: Story = {
  args: { defaultChecked: true, disabled: true },
}

/** No label: the box alone, named through `aria-label` (a table's row selector). */
export const Bare: Story = {
  args: { label: undefined, 'aria-label': 'Select row' },
}

function ControlledPreview() {
  const [checked, setChecked] = useState(true)
  return (
    <Checkbox
      label="Follow output"
      checked={checked}
      onChange={(event) => setChecked(event.currentTarget.checked)}
    />
  )
}

export const Controlled: Story = {
  render: () => <ControlledPreview />,
}
