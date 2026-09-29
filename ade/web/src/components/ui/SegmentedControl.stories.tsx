import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { SegmentedControl, type SegmentedControlProps } from './ModeToggle'

type DemoProps = Partial<
  Omit<SegmentedControlProps<string>, 'value' | 'onChange'>
>

/** Values map to the semantic tab icons (`TabIcon`); `icon: false` opts out. */
const views: SegmentedControlProps<string>['options'] = [
  { value: 'overview', label: 'Overview' },
  { value: 'logs', label: 'Logs' },
  { value: 'files', label: 'Files' },
  { value: 'config', label: 'Config' },
]

function Demo({ options = views, ...props }: DemoProps) {
  const [value, setValue] = useState(options[0].value)
  return (
    <SegmentedControl
      value={value}
      onChange={setValue}
      options={options}
      {...props}
    />
  )
}

const meta = {
  title: 'UI/SegmentedControl',
  component: Demo,
  parameters: { layout: 'padded' },
} satisfies Meta<typeof Demo>

export default meta
type Story = StoryObj<typeof meta>

/** `tabs` (default): a tablist for switching views. */
export const Tabs: Story = {}

/** Only the icon renders; each label moves into a tooltip. */
export const IconOnly: Story = {
  args: { iconOnly: true, 'aria-label': 'View' },
}

export const NoIcons: Story = {
  args: { options: views.map((view) => ({ ...view, icon: false })) },
}

/** `radio`: a radiogroup for one persistent preference; never shows icons. */
export const Radio: Story = {
  args: {
    variant: 'radio',
    'aria-label': 'Theme',
    options: [
      { value: 'system', label: 'System' },
      { value: 'light', label: 'Light' },
      { value: 'dark', label: 'Dark' },
    ],
  },
}
