import type { Meta, StoryObj } from '@storybook/react-vite'
import { fn } from 'storybook/test'
import { Button } from './Button'
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
  DialogTrigger,
} from './Dialog'
import { Input } from './Input'

const meta = {
  title: 'UI/Dialog',
  component: Dialog,
  parameters: { layout: 'centered' },
} satisfies Meta<typeof Dialog>

export default meta
type Story = StoryObj<typeof meta>

const notes = Array.from(
  { length: 24 },
  (_, i) => `0.${i + 1}.0 — maintenance release, no breaking changes.`,
)

/** Opened on mount so the canvas shows the panel; the trigger reopens it. */
export const Default: Story = {
  render: () => (
    <Dialog defaultOpen>
      <DialogTrigger asChild>
        <Button variant="pill" size="sm">
          Rename worker
        </Button>
      </DialogTrigger>
      <DialogContent className="max-w-md">
        <DialogTitle>Rename worker</DialogTitle>
        <DialogDescription>
          The new name shows in the sidebar and in trace attribution.
        </DialogDescription>
        <div className="mt-4">
          <Input value="browser" onChange={fn()} aria-label="Worker name" />
        </div>
        <div className="mt-6 flex justify-end gap-2">
          <DialogClose asChild>
            <Button variant="ghost" size="sm">
              Cancel
            </Button>
          </DialogClose>
          <DialogClose asChild>
            <Button variant="primary" size="sm">
              Save
            </Button>
          </DialogClose>
        </div>
      </DialogContent>
    </Dialog>
  ),
}

/** Bodies taller than 85vh scroll inside the panel. */
export const Scrollable: Story = {
  render: () => (
    <Dialog defaultOpen>
      <DialogTrigger asChild>
        <Button variant="pill" size="sm">
          Release notes
        </Button>
      </DialogTrigger>
      <DialogContent>
        <DialogTitle>Release notes</DialogTitle>
        <DialogDescription>Every release since 0.1.0.</DialogDescription>
        <ul className="mt-4 flex flex-col gap-2 font-sans text-[12px] text-ink">
          {notes.map((note) => (
            <li key={note}>{note}</li>
          ))}
        </ul>
      </DialogContent>
    </Dialog>
  ),
}
