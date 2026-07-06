import type { Meta, StoryObj } from '@storybook/react-vite'
import { ServiceBreakdown } from '../components/ServiceBreakdown'
import {
  WATERFALL_FIXTURE,
  WATERFALL_SIMPLE,
} from '../fixtures/traces-fixtures'

const meta = {
  title: 'TracesV2/ServiceBreakdown',
  component: ServiceBreakdown,
  parameters: { layout: 'centered' },
  decorators: [
    (Story) => (
      <div className="w-[560px] border border-rule bg-panel">
        <Story />
      </div>
    ),
  ],
  args: { data: WATERFALL_FIXTURE },
} satisfies Meta<typeof ServiceBreakdown>

export default meta
type Story = StoryObj<typeof meta>

export const Default: Story = {}

export const SimpleTrace: Story = { args: { data: WATERFALL_SIMPLE } }
