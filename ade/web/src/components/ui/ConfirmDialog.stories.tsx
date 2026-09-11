import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { Button } from './Button'
import { ConfirmDialog } from './ConfirmDialog'

function Demo({ details }: { details?: string[] }) {
  const [open, setOpen] = useState(true)
  const [outcome, setOutcome] = useState<string>('')
  return (
    <div className="flex flex-col items-start gap-3">
      <Button variant="pill" size="sm" onClick={() => setOpen(true)}>
        Close workspace
      </Button>
      <span className="font-sans text-[12px] text-ink-faint">{outcome}</span>
      <ConfirmDialog
        open={open}
        onOpenChange={setOpen}
        title="Close this workspace?"
        description="Unsaved work in it will be lost."
        details={details}
        confirmLabel="Close and discard"
        cancelLabel="Keep working"
        onConfirm={() => setOutcome('Closed and discarded.')}
        onCancel={() => setOutcome('Kept working.')}
      />
    </div>
  )
}

const meta = {
  title: 'UI/ConfirmDialog',
  component: Demo,
  parameters: { layout: 'padded' },
} satisfies Meta<typeof Demo>

export default meta
type Story = StoryObj<typeof meta>

export const UnsavedWork: Story = {
  args: { details: ['src/main.rs', 'notes/release.md'] },
}

export const Plain: Story = {
  args: {},
}
