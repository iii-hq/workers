import type { Meta, StoryObj } from '@storybook/react-vite'
import { Skeleton } from './Skeleton'

const meta = {
  title: 'UI/Skeleton',
  component: Skeleton,
  parameters: { layout: 'padded' },
} satisfies Meta<typeof Skeleton>

export default meta
type Story = StoryObj<typeof meta>

/** Size it with utilities; the pulse and surface fill come from the component. */
export const Default: Story = { args: { className: 'h-3 w-40' } }

/** Inline in a sentence the values are still loading into. */
export const Inline: Story = {
  render: () => (
    <p className="font-sans text-[12px] text-ink-faint">
      Last run took <Skeleton className="h-3 w-12" /> on{' '}
      <Skeleton className="h-3 w-20" />.
    </p>
  ),
}

/** A list-row placeholder: avatar disc plus two lines of copy. */
export const Rows: Story = {
  render: () => (
    <div className="flex w-72 flex-col gap-3">
      {['a', 'b', 'c'].map((row) => (
        <div key={row} className="flex items-center gap-3">
          <Skeleton className="size-8 rounded-full" />
          <div className="flex flex-1 flex-col gap-1.5">
            <Skeleton className="h-3 w-[60%]" />
            <Skeleton className="h-2.5 w-4/5" />
          </div>
        </div>
      ))}
    </div>
  ),
}
