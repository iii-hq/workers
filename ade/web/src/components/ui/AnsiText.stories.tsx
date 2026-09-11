import type { Meta, StoryObj } from '@storybook/react-vite'
import { AnsiText } from './AnsiText'

const ESC = '\u001b'

const COLOR_MAP = [
  `${ESC}[31mred → alert${ESC}[0m`,
  `${ESC}[32mgreen → ok${ESC}[0m`,
  `${ESC}[33myellow → warn${ESC}[0m`,
  `${ESC}[34mblue → accent${ESC}[0m`,
  `${ESC}[35mmagenta → accent${ESC}[0m`,
  `${ESC}[36mcyan → accent${ESC}[0m`,
  `${ESC}[91mbright red → alert${ESC}[0m`,
  `${ESC}[1mbold → semibold${ESC}[0m`,
  `${ESC}[1;31mbold red${ESC}[0m`,
  `${ESC}[38;5;196m256-color params consumed (default ink)${ESC}[0m`,
  `${ESC}[38;2;255;0;0mtruecolor params consumed (default ink)${ESC}[0m`,
].join('\n')

const TEST_RUN = [
  `${ESC}[1mrunning 3 tests${ESC}[0m`,
  `test parse::roundtrip ... ${ESC}[32mok${ESC}[0m`,
  `test parse::extended ... ${ESC}[32mok${ESC}[0m`,
  `test parse::malformed ... ${ESC}[31mFAILED${ESC}[0m`,
  '',
  `${ESC}[33mwarning${ESC}[0m: unused variable \`state\``,
  `${ESC}[36m--> ${ESC}[0msrc/lib/ansi.ts:42:9`,
].join('\n')

const meta = {
  title: 'UI/AnsiText',
  component: AnsiText,
  parameters: { layout: 'padded' },
  // The component inherits its font — stories wrap it in the mono pane a
  // real caller (TerminalStream) provides.
  render: (args) => (
    <pre className="m-0 whitespace-pre-wrap font-mono text-[12.5px] leading-[1.55] text-ink">
      <AnsiText {...args} />
    </pre>
  ),
} satisfies Meta<typeof AnsiText>

export default meta
type Story = StoryObj<typeof meta>

/** Every SGR code the parser maps, one line each. */
export const ColorMapping: Story = { args: { text: COLOR_MAP } }

/** What real tool output looks like through the mapping. */
export const TestRun: Story = { args: { text: TEST_RUN } }

/** Escapes outside the subset are stripped, never leaked. */
export const StrippedSequences: Story = {
  args: {
    text: `${ESC}]0;window title\u0007cleared${ESC}[2J the screen, moved ${ESC}[1;1Hthe cursor — none of it visible`,
  },
}
