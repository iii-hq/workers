import type { Meta, StoryObj } from '@storybook/react-vite'
import { useLiveAnnouncer } from '@/hooks/use-live-announcer'
import { Button } from './Button'
import { LiveRegion } from './LiveRegion'

const meta = {
  title: 'UI/LiveRegion',
  component: LiveRegion,
  parameters: { layout: 'padded' },
} satisfies Meta<typeof LiveRegion>

export default meta
type Story = StoryObj<typeof meta>

/** Visually hidden; turn on a screen reader (or inspect the DOM) — each
    click re-announces even when the text repeats. */
export const Announcer: Story = {
  args: { announcement: null },
  render: () => {
    const { announcement, announce, announceAssertive } = useLiveAnnouncer()
    return (
      <div className="flex items-center gap-2">
        <Button type="button" onClick={() => announce('auto-accepted: fs::ls')}>
          Announce politely
        </Button>
        <Button
          type="button"
          variant="pill"
          onClick={() => announceAssertive('worker disconnected')}
        >
          Announce assertively
        </Button>
        <LiveRegion announcement={announcement} />
        <span className="font-mono text-[11px] text-ink-faint">
          last: {announcement?.text ?? '—'} (seq {announcement?.seq ?? 0})
        </span>
      </div>
    )
  },
}
