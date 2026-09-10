import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { ViewSwitcher, type ViewType } from '../components/ViewSwitcher'

// `args` can't hold React state, so a small wrapper owns the controlled
// `currentView` and feeds `onViewChange` back into it — the toggle actually
// switches in the playground.
function ViewSwitcherHarness() {
  const [currentView, setCurrentView] = useState<ViewType>('waterfall')
  return (
    <ViewSwitcher currentView={currentView} onViewChange={setCurrentView} />
  )
}

const meta = {
  title: 'TracesV2/ViewSwitcher',
  component: ViewSwitcherHarness,
  parameters: { layout: 'centered' },
  decorators: [
    (Story) => (
      <div className="p-4 bg-bg border border-rule inline-block">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ViewSwitcherHarness>

export default meta
type Story = StoryObj<typeof meta>

export const Default: Story = {}
