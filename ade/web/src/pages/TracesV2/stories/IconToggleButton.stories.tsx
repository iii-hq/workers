import type { Meta, StoryObj } from '@storybook/react-vite'
import { Eye } from 'lucide-react'
import { useState } from 'react'
import { IconToggleButton } from '../components/IconToggleButton'

// `active` is a controlled prop, so Storybook args can't drive the toggle on
// their own. This wrapper owns the state, seeds it from `active`, and renders
// the real button; the exported stories tune the seed + label through args.
function IconToggleButtonHarness({
  active = false,
  label,
}: {
  active?: boolean
  label: string
}) {
  const [on, setOn] = useState(active)
  return (
    <div className="p-4 bg-bg border border-rule inline-block">
      <IconToggleButton
        active={on}
        onClick={() => setOn((v) => !v)}
        label={label}
      >
        <Eye className="w-4 h-4" />
      </IconToggleButton>
    </div>
  )
}

const meta = {
  title: 'TracesV2/IconToggleButton',
  component: IconToggleButtonHarness,
  parameters: { layout: 'centered' },
  args: { label: 'toggle view' },
} satisfies Meta<typeof IconToggleButtonHarness>

export default meta
type Story = StoryObj<typeof meta>

export const Default: Story = {}

export const Active: Story = { args: { active: true } }
