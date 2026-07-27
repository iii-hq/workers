import type { Meta, StoryObj } from '@storybook/react-vite'
import { fn } from 'storybook/test'
import { SpanBaggageTab } from '../components/SpanBaggageTab'
import { SpanErrorsTab } from '../components/SpanErrorsTab'
import { SpanInfoTab } from '../components/SpanInfoTab'
import { SpanLinksTab } from '../components/SpanLinksTab'
import { SpanLogsTab } from '../components/SpanLogsTab'
import { SpanOtelLogsTab } from '../components/SpanOtelLogsTab'
import { SpanTagsTab } from '../components/SpanTagsTab'
import {
  ERROR_SPAN,
  FN_SPAN,
  LLM_SPAN,
  WATERFALL_FIXTURE,
} from '../fixtures/traces-fixtures'
import { withFakeIiiClient } from './harness'

const meta = {
  title: 'TracesV2/SpanTabs',
  component: SpanInfoTab,
  parameters: { layout: 'centered' },
  // Each story renders a specific tab via `render`; these default args just
  // satisfy meta.component (SpanInfoTab) so stories don't each need their own.
  args: { span: FN_SPAN, traceData: WATERFALL_FIXTURE },
} satisfies Meta<typeof SpanInfoTab>
export default meta
type Story = StoryObj<typeof meta>

/**
 * FN_SPAN is a function invocation (`faas.invoked_name` + `iii.payload.json`
 * events), so the info tab leads with the shared FunctionTriggerCard — the same
 * card chat renders — showing the request/response payloads.
 */
export const Info: Story = {
  render: () => (
    <div className="w-[440px] border border-rule bg-panel p-0">
      <SpanInfoTab span={FN_SPAN} traceData={WATERFALL_FIXTURE} />
    </div>
  ),
}

/** A non-function span (llm client call): no call card, just the span facts. */
export const InfoNonFunction: Story = {
  render: () => (
    <div className="w-[440px] border border-rule bg-panel p-0">
      <SpanInfoTab span={LLM_SPAN} traceData={WATERFALL_FIXTURE} />
    </div>
  ),
}

export const Tags: Story = {
  render: () => (
    <div className="w-[440px] border border-rule bg-panel p-0">
      <SpanTagsTab span={LLM_SPAN} />
    </div>
  ),
}

export const Logs: Story = {
  render: () => (
    <div className="w-[440px] border border-rule bg-panel p-0">
      <SpanLogsTab span={LLM_SPAN} />
    </div>
  ),
}

export const Errors: Story = {
  render: () => (
    <div className="w-[440px] border border-rule bg-panel p-0">
      <SpanErrorsTab span={ERROR_SPAN} />
    </div>
  ),
}

export const Baggage: Story = {
  render: () => (
    <div className="w-[440px] border border-rule bg-panel p-0">
      <SpanBaggageTab span={FN_SPAN} />
    </div>
  ),
}

export const Links: Story = {
  render: () => (
    <div className="w-[440px] border border-rule bg-panel p-0">
      <SpanLinksTab
        span={LLM_SPAN}
        onNavigateToTrace={fn()}
        onNavigateToSpan={fn()}
      />
    </div>
  ),
}

export const OtelLogs: Story = {
  decorators: [withFakeIiiClient],
  render: () => (
    <div className="w-[440px] border border-rule bg-panel p-0">
      <SpanOtelLogsTab span={FN_SPAN} />
    </div>
  ),
}
