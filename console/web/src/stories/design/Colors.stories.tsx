import type { Meta, StoryObj } from '@storybook/react-vite'
import { cn } from '@/lib/utils'

interface Swatch {
  token: string
  className: string
  note: string
}

const PAPER: Swatch[] = [
  { token: 'bg', className: 'bg-bg', note: 'page paper' },
  {
    token: 'panel',
    className: 'bg-panel',
    note: 'header strips, focused cards',
  },
  { token: 'paper-2', className: 'bg-paper-2', note: 'nested separation' },
]

const INK: Swatch[] = [
  { token: 'ink', className: 'bg-ink', note: 'primary type, wordmark' },
  {
    token: 'ink-faint',
    className: 'bg-ink-faint',
    note: 'muted body, inactive',
  },
  {
    token: 'ink-ghost',
    className: 'bg-ink-ghost',
    note: 'placeholders, line numbers',
  },
]

const RULES: Swatch[] = [
  { token: 'rule', className: 'bg-rule', note: 'default 1px borders' },
  { token: 'rule-2', className: 'bg-rule-2', note: 'nested separators' },
]

const STATUS: Swatch[] = [
  { token: 'accent', className: 'bg-accent', note: 'single hero accent' },
  { token: 'alert', className: 'bg-alert', note: 'error states' },
  { token: 'warn', className: 'bg-warn', note: 'warning states' },
]

function SwatchRow({ list, label }: { list: Swatch[]; label: string }) {
  return (
    <div>
      <div className="font-mono text-[11px] uppercase tracking-[0.18em] text-ink-faint mb-3">
        {label}
      </div>
      <div className="grid grid-cols-2 @3xl:grid-cols-3 gap-3 @container">
        {list.map((s) => (
          <div key={s.token} className="border border-rule bg-bg">
            <div className={cn('h-12 border-b border-rule-2', s.className)} />
            <div className="px-3 py-2 flex flex-col gap-0.5">
              <span className="font-mono text-[12px] text-ink">{s.token}</span>
              <span className="font-mono text-[11px] text-ink-faint lowercase">
                {s.note}
              </span>
            </div>
          </div>
        ))}
      </div>
    </div>
  )
}

const meta = {
  title: 'Design/Colors',
  parameters: { layout: 'padded' },
} satisfies Meta

export default meta
type Story = StoryObj

export const Tokens: Story = {
  render: () => (
    <div className="flex flex-col gap-8">
      <SwatchRow list={PAPER} label="paper" />
      <SwatchRow list={INK} label="ink" />
      <SwatchRow list={RULES} label="rules" />
      <SwatchRow list={STATUS} label="accent + status" />
    </div>
  ),
}
