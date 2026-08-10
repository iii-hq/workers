import type { Meta, StoryObj } from '@storybook/react-vite'
import { Caret } from '@/components/ui/Caret'
import { StatusDot } from '@/components/ui/StatusDot'
import { cn } from '@/lib/utils'

interface TraceRow {
  id: string
  op: string
  durationMs: number
  startMs: number
  spanMs: number
  status: 'ok' | 'warn' | 'err'
}

const traceRows: TraceRow[] = [
  {
    id: '1',
    op: 'router::dispatch',
    startMs: 0,
    spanMs: 18,
    durationMs: 18,
    status: 'ok',
  },
  {
    id: '2',
    op: 'engine::echo',
    startMs: 20,
    spanMs: 132,
    durationMs: 132,
    status: 'ok',
  },
  {
    id: '3',
    op: 'functions::info',
    startMs: 160,
    spanMs: 88,
    durationMs: 88,
    status: 'warn',
  },
  {
    id: '4',
    op: 'directory::index',
    startMs: 250,
    spanMs: 410,
    durationMs: 410,
    status: 'err',
  },
]

const traceTone: Record<
  TraceRow['status'],
  { dot: 'accent' | 'warn' | 'alert'; bar: string; label: string }
> = {
  ok: { dot: 'accent', bar: 'bg-ink', label: 'text-accent' },
  warn: { dot: 'warn', bar: 'bg-warn', label: 'text-warn' },
  err: { dot: 'alert', bar: 'bg-alert', label: 'text-alert' },
}

function Card({
  label,
  children,
}: {
  label: string
  children: React.ReactNode
}) {
  return (
    <div className="border border-rule bg-bg">
      <div className="bg-panel px-3 py-1.5 border-b border-rule-2 font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
        {label}
      </div>
      <div className="p-4">{children}</div>
    </div>
  )
}

const meta = {
  title: 'Design/Loading',
  parameters: { layout: 'padded' },
} satisfies Meta

export default meta
type Story = StoryObj

export const StatusDots: Story = {
  render: () => (
    <Card label="status dots">
      <div className="flex flex-wrap items-center gap-x-6 gap-y-3 font-mono text-[12px] text-ink-faint lowercase">
        <span className="flex items-center gap-2">
          <StatusDot tone="accent" pulse /> live
        </span>
        <span className="flex items-center gap-2">
          <StatusDot tone="accent" /> ok
        </span>
        <span className="flex items-center gap-2">
          <StatusDot tone="warn" /> warn
        </span>
        <span className="flex items-center gap-2">
          <StatusDot tone="alert" /> alert
        </span>
        <span className="flex items-center gap-2">
          <StatusDot tone="ink" /> idle
        </span>
      </div>
    </Card>
  ),
}

export const BlinkingCaret: Story = {
  render: () => (
    <Card label="blinking caret">
      <div className="font-mono text-[14px] text-ink leading-[1.7]">
        triggering engine::echo
        <Caret className="ml-1" />
      </div>
    </Card>
  ),
}

export const ThinkingPlaceholder: Story = {
  render: () => (
    <Card label="thinking placeholder">
      <div className="font-mono text-[13px] text-ink-faint italic leading-[1.7]">
        thinking…
      </div>
    </Card>
  ),
}

export const StreamingTokenTail: Story = {
  render: () => (
    <Card label="streaming token tail">
      <div className="font-mono text-[14px] text-ink leading-[1.7]">
        sure — a btree-backed index gives you both{' '}
        <code className="font-mono text-[12.5px] border border-rule-2 bg-paper-2 text-ink px-1">
          O(log&nbsp;n)
        </code>{' '}
        lookups and cheap range
        <Caret className="ml-0.5" />
      </div>
    </Card>
  ),
}

export const TraceWaterfall: Story = {
  render: () => {
    const totalMs = traceRows.reduce(
      (m, r) => Math.max(m, r.startMs + r.spanMs),
      0,
    )
    return (
      <Card label="trace waterfall">
        <ul className="divide-y divide-rule-2 -mx-4 -my-4">
          {traceRows.map((row) => {
            const tone = traceTone[row.status]
            const start = (row.startMs / totalMs) * 100
            const width = (row.spanMs / totalMs) * 100
            return (
              <li
                key={row.id}
                className="grid grid-cols-[auto_1fr_auto_auto] items-center gap-x-3 px-4 py-2 font-mono text-[12px]"
              >
                <StatusDot
                  tone={tone.dot}
                  pulse={row.status === 'ok' && row.id === '2'}
                />
                <span className="text-ink truncate lowercase">{row.op}</span>
                <span className="text-ink-faint tabular-nums">
                  {row.durationMs}ms
                </span>
                <span
                  className={cn(
                    'text-[11px] uppercase tracking-[0.06em] font-medium',
                    tone.label,
                  )}
                >
                  {row.status}
                </span>
                <div className="col-span-4 mt-1 h-1 bg-surface relative">
                  <div
                    className={cn('absolute top-0 h-full', tone.bar)}
                    style={{ left: `${start}%`, width: `${width}%` }}
                  />
                </div>
              </li>
            )
          })}
        </ul>
      </Card>
    )
  },
}
