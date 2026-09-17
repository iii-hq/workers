import type { Meta, StoryObj } from '@storybook/react-vite'
import { ErrorBoundary } from './ErrorBoundary'
import { StatusPanel } from './StatusPanel'

/** A child that throws during render, the way a broken worker page would. */
function Broken(): never {
  throw new Error("Cannot read properties of undefined (reading 'sessions')")
}

const meta = {
  title: 'UI/ErrorBoundary',
  component: ErrorBoundary,
  parameters: { layout: 'padded' },
  decorators: [
    (Story) => (
      <div className="max-w-md">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ErrorBoundary>

export default meta
type Story = StoryObj<typeof meta>

/** The default fallback is an alert StatusPanel carrying the error message. */
export const DefaultFallback: Story = {
  args: { children: <Broken /> },
}

/** `fallback` swaps in any surface; it receives the caught error. */
export const CustomFallback: Story = {
  args: {
    children: <Broken />,
    fallback: (error) => (
      <StatusPanel
        variant="warn"
        headline="This pane crashed"
        detail={`${error.message} — reload the worker page to retry.`}
      />
    ),
  },
}

export const Healthy: Story = {
  args: {
    children: (
      <p className="font-sans text-[12px] text-ink">
        Children render untouched until something throws.
      </p>
    ),
  },
}
