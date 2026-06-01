import type { Meta, StoryObj } from '@storybook/react-vite'

interface TypeRow {
  token: string
  className: string
  sample: string
  spec: string
}

const ROWS: TypeRow[] = [
  {
    token: 'display-hero',
    className:
      'font-mono text-[72px] font-semibold tracking-[-0.02em] leading-[1.02] text-ink',
    sample: 'any task. one experience.',
    spec: '72px / 600',
  },
  {
    token: 'display-foot',
    className:
      'font-mono text-[48px] font-semibold tracking-[-0.03em] leading-[1.05] text-ink',
    sample: 'work the plan.',
    spec: '48px / 600',
  },
  {
    token: 'headline-section',
    className:
      'font-mono text-[28px] font-medium tracking-[-0.01em] leading-[1.25] text-ink',
    sample: 'lines define structure.',
    spec: '28px / 500',
  },
  {
    token: 'headline-card',
    className:
      'font-mono text-[20px] font-medium tracking-[-0.02em] leading-[1.1] text-ink',
    sample: 'an engineering document, not a dashboard.',
    spec: '20px / 500',
  },
  {
    token: 'title-cell',
    className:
      'font-mono text-[16px] font-semibold tracking-[-0.01em] leading-[1.3] text-ink',
    sample: 'cell title — 16px semibold.',
    spec: '16px / 600',
  },
  {
    token: 'body-md',
    className: 'font-mono text-[14px] leading-[1.7] text-ink',
    sample:
      'the body copy is monospace. variety comes from weight, scale, case, and letter-spacing — not family.',
    spec: '14px / 400',
  },
  {
    token: 'body-sm',
    className: 'font-mono text-[13px] leading-[1.7] text-ink-faint',
    sample:
      'compact body in ink-faint. used inside cards, sidebars, and meta rows.',
    spec: '13px / 400',
  },
  {
    token: 'code-md',
    className: 'font-mono text-[13px] leading-[1.65] text-ink',
    sample: 'await dispatcher.run({ functionId: "engine::echo" })',
    spec: '13px / 400 (code)',
  },
  {
    token: 'code-sm',
    className: 'font-mono text-[12.5px] leading-[1.55] text-ink',
    sample: 'await dispatcher.run({ functionId: "engine::echo" })',
    spec: '12.5px / 400 (code)',
  },
  {
    token: 'label-caps-lg',
    className:
      'font-mono text-[12px] font-medium uppercase tracking-[0.18em] text-ink-faint',
    sample: 'section eyebrow',
    spec: '12px / 500 upper',
  },
  {
    token: 'label-caps-md',
    className:
      'font-mono text-[12px] font-medium uppercase tracking-[0.14em] text-ink-faint',
    sample: 'card head',
    spec: '12px / 500 upper',
  },
  {
    token: 'label-caps-sm',
    className:
      'font-mono text-[11px] font-medium uppercase tracking-[0.06em] text-ink-faint',
    sample: 'meta row',
    spec: '11px / 500 upper',
  },
  {
    token: 'micro',
    className: 'font-mono text-[9px] tracking-[0.04em] text-ink-faint',
    sample: 'diagram annotation',
    spec: '9px / 400',
  },
]

const meta = {
  title: 'Design/Typography',
  parameters: { layout: 'padded' },
} satisfies Meta

export default meta
type Story = StoryObj

export const Scale: Story = {
  render: () => (
    <div className="border border-rule bg-bg divide-y divide-rule-2">
      {ROWS.map((row) => (
        <div
          key={row.token}
          className="grid grid-cols-[200px_1fr_auto] items-baseline gap-x-6 px-4 py-4"
        >
          <div className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
            {row.token}
          </div>
          <div className={row.className}>{row.sample}</div>
          <div className="font-mono text-[11px] text-ink-ghost tabular-nums">
            {row.spec}
          </div>
        </div>
      ))}
    </div>
  ),
}
