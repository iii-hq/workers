import type { Meta, StoryObj } from '@storybook/react-vite'
import { TerminalStream } from './TerminalStream'

const ESC = '\u001b'

const LONG = Array.from(
  { length: 30 },
  (_, i) => `[${String(i + 1).padStart(2, '0')}] resolving module graph…`,
).join('\n')

const STDERR = [
  'error: linking with `cc` failed: exit status: 1',
  '  = note: ld: library not found for -lssl',
  'error: could not compile `worker` (bin "worker") due to 1 previous error',
].join('\n')

const ANSI_RUN = [
  `${ESC}[1m$ vitest run${ESC}[0m`,
  `${ESC}[32m✓${ESC}[0m src/lib/ansi.test.ts (18 tests)`,
  `${ESC}[32m✓${ESC}[0m src/components/ui/TerminalStream.test.tsx (11 tests)`,
  `${ESC}[31m✗${ESC}[0m src/flaky.test.ts — ${ESC}[33m1 retry${ESC}[0m`,
].join('\n')

const meta = {
  title: 'UI/TerminalStream',
  component: TerminalStream,
  parameters: { layout: 'padded' },
  argTypes: {
    tone: { control: 'select', options: ['out', 'err'] },
    ansi: { control: 'boolean' },
  },
} satisfies Meta<typeof TerminalStream>

export default meta
type Story = StoryObj<typeof meta>

/** 30 lines against the default 12-line clamp — the toggle is live. */
export const ClampAndExpand: Story = {
  args: { label: 'stdout', text: LONG },
}

/** Warn tint: stderr is the user's program failing, not the console. */
export const ErrTone: Story = {
  args: { label: 'stderr', tone: 'err', text: STDERR },
}

/** ANSI colors mapped through AnsiText. */
export const AnsiBody: Story = {
  args: { label: 'stdout', ansi: true, text: ANSI_RUN },
}

/** Stdout stacked above stderr, the exec-card shape. */
export const StreamPair: Story = {
  args: { label: 'stdout', text: 'build ok · 42 modules' },
  render: (args) => (
    <div className="flex flex-col gap-3">
      <TerminalStream {...args} />
      <TerminalStream label="stderr" tone="err" text={STDERR} />
    </div>
  ),
}
