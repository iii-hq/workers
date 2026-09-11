import type { Meta, StoryObj } from '@storybook/react-vite'
import { fn } from 'storybook/test'
import { SpanPanel } from '../components/SpanPanel'
import {
  ERROR_SPAN,
  FN_SPAN,
  LLM_SPAN,
  ROOT_SPAN,
  WATERFALL_FIXTURE,
} from '../fixtures/traces-fixtures'
import { LabFrame, withFakeIiiClient } from './harness'

const meta = {
  title: 'TracesV2/SpanPanel',
  component: SpanPanel,
  parameters: { layout: 'centered' },
  decorators: [
    withFakeIiiClient,
    (Story) => (
      <LabFrame className="h-[620px] w-[460px]">
        <Story />
      </LabFrame>
    ),
  ],
  args: {
    traceData: WATERFALL_FIXTURE,
    onClose: fn(),
    onNavigateToSpan: fn(),
    onNavigateToTrace: fn(),
  },
} satisfies Meta<typeof SpanPanel>

export default meta

type Story = StoryObj<typeof meta>

export const ErrorSpan: Story = { args: { span: ERROR_SPAN } }

export const LlmSpan: Story = { args: { span: LLM_SPAN } }

export const RootSpan: Story = { args: { span: ROOT_SPAN } }

export const FnSpan: Story = { args: { span: FN_SPAN } }
