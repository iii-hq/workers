import type { Meta, StoryObj } from '@storybook/react-vite'
import type { ReactNode } from 'react'
import { EdgeAddZone, SplitPreview } from './pane-controls'

interface WorkspaceRowProps {
  columns: number
  width?: number
  children?: ReactNode
}

/** A stand-in for the workspace row: the canvas, its 16px gutters and the
    floating panels — the geometry App.tsx gives the real edge zones. */
function WorkspaceRow({ columns, width = 760, children }: WorkspaceRowProps) {
  return (
    <div
      className="relative flex h-[26rem] gap-1.5 bg-bg px-4 pb-1.5"
      style={{ width }}
    >
      {Array.from({ length: columns }, (_, index) => (
        <section
          // biome-ignore lint/suspicious/noArrayIndexKey: position is the identity
          key={index}
          aria-label={`panel ${index + 1} of ${columns}`}
          className="flex min-w-0 flex-1 flex-col overflow-hidden rounded-sm border border-edge bg-panel"
        >
          <div className="flex h-10 shrink-0 items-center bg-panel-raised px-3 font-sans text-[12px] font-semibold text-ink">
            Panel {index + 1}
          </div>
          <div className="flex flex-col gap-2.5 p-3">
            <span className="h-2.5 w-2/3 rounded-sm bg-surface" />
            <span className="h-2.5 w-full rounded-sm bg-surface" />
            <span className="h-2.5 w-5/6 rounded-sm bg-surface" />
            <span className="h-2.5 w-1/2 rounded-sm bg-surface" />
          </div>
        </section>
      ))}
      {children}
    </div>
  )
}

interface ZonesProps {
  columns: number
  nudge: boolean
  disabled: boolean
}

function Zones({ columns, nudge, disabled }: ZonesProps) {
  return (
    <div className="flex flex-col gap-4">
      <WorkspaceRow columns={columns}>
        <EdgeAddZone
          side="left"
          columns={columns}
          nudge={nudge}
          disabled={disabled}
          onAdd={() => {}}
        />
        <EdgeAddZone
          side="right"
          columns={columns}
          nudge={nudge}
          disabled={disabled}
          onAdd={() => {}}
        />
      </WorkspaceRow>
      <p className="max-w-[60ch] font-sans text-[12px] text-ink-faint">
        Rest the pointer on either edge of the canvas for a beat, or click it,
        to reveal the split preview. Tab to an edge and press Enter to open it
        from the keyboard; Escape closes it.
      </p>
    </div>
  )
}

const meta = {
  title: 'Workspace/PaneControls',
  component: Zones,
  args: { columns: 2, nudge: false, disabled: false },
  argTypes: {
    columns: { control: { type: 'range', min: 1, max: 4, step: 1 } },
  },
} satisfies Meta<typeof Zones>

export default meta
type Story = StoryObj<typeof meta>

/** After the first split the edges are bare: nothing is drawn until a dwell,
    a click or a keyboard activation reveals the preview. */
export const Discovered: Story = {}

/** Before the first split the edges keep a framed `+` sliver that shakes
    once every ten seconds, and the preview mentions the other edge. */
export const FirstRun: Story = {
  args: { columns: 1, nudge: true },
}

/** Both previews held open, the way a dwell shows them. */
export const Preview: StoryObj<{ columns: number; nudge: boolean }> = {
  args: { columns: 2, nudge: false },
  argTypes: {
    columns: { control: { type: 'range', min: 1, max: 6, step: 1 } },
  },
  render: ({ columns, nudge }) => (
    <WorkspaceRow columns={columns}>
      <div className="absolute inset-y-0 left-0 z-20 flex w-32 pb-1.5">
        <SplitPreview
          side="left"
          columns={columns}
          nudge={nudge}
          onAdd={() => {}}
        />
      </div>
      <div className="absolute inset-y-0 right-0 z-20 flex w-32 pb-1.5">
        <SplitPreview
          side="right"
          columns={columns}
          nudge={nudge}
          onAdd={() => {}}
        />
      </div>
    </WorkspaceRow>
  ),
}
