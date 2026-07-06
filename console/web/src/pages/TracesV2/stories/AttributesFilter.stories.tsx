import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { AttributesFilter } from '../components/AttributesFilter'

// `AttributesFilter` is a controlled `value` + `onChange` pair: Storybook args
// can't hold React state, so this wrapper owns the applied pairs and feeds them
// back down. The component keeps its own `draft` and re-seeds it whenever the
// `value` identity changes, so pushing a fresh array on apply is enough.
function AttributesFilterHarness({
  value = [],
}: {
  value?: [string, string][]
}) {
  const [attrs, setAttrs] = useState<[string, string][]>(value)
  return (
    <div className="w-[520px] p-4 bg-bg border border-rule">
      <AttributesFilter value={attrs} onChange={setAttrs} />
    </div>
  )
}

const meta = {
  title: 'TracesV2/AttributesFilter',
  component: AttributesFilterHarness,
  parameters: { layout: 'centered' },
} satisfies Meta<typeof AttributesFilterHarness>

export default meta
type Story = StoryObj<typeof meta>

export const Empty: Story = { args: { value: [] } }

export const WithRows: Story = {
  args: {
    value: [
      ['service.name', 'agent'],
      ['http.method', 'POST'],
    ],
  },
}
