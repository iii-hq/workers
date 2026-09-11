import type { Meta, StoryObj } from '@storybook/react-vite'
import { Badge } from './Badge'
import { TerminalCommandLine } from './TerminalCommandLine'

const meta = {
  title: 'UI/TerminalCommandLine',
  component: TerminalCommandLine,
  parameters: { layout: 'padded' },
  args: { command: 'cargo test --workspace' },
  argTypes: {
    copy: { control: 'boolean' },
  },
} satisfies Meta<typeof TerminalCommandLine>

export default meta
type Story = StoryObj<typeof meta>

export const Plain: Story = {}

/** Trailing chips + the copy affordance (click it — copied/failed flash). */
export const ChipsAndCopy: Story = {
  args: {
    command: 'pnpm build && pnpm test',
    copy: true,
    chips: (
      <>
        <Badge variant="accent">exit 0</Badge>
        <Badge>1.2s</Badge>
      </>
    ),
  },
}

/** The command ellipsizes; the full text rides on the hover title. */
export const LongCommand: Story = {
  args: {
    command:
      'curl -fsSL https://install.iii.dev/iii/main/install.sh | sh -s -- --next --with-a-very-long-flag-list --that-cannot-possibly-fit',
    copy: true,
  },
  render: (args) => (
    <div className="max-w-[360px] border border-rule-2 bg-paper-2 px-3 py-2">
      <TerminalCommandLine {...args} />
    </div>
  ),
}

/** A different prompt glyph. */
export const CustomPrompt: Story = {
  args: { command: 'kubectl get pods -A', prompt: '❯' },
}
