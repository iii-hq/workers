import type { Meta, StoryObj } from '@storybook/react-vite'
import { Copy, Pencil, Trash2 } from 'lucide-react'
import { Button } from './Button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from './DropdownMenu'

const meta = {
  title: 'UI/DropdownMenu',
  component: DropdownMenu,
  parameters: { layout: 'centered' },
} satisfies Meta<typeof DropdownMenu>

export default meta
type Story = StoryObj<typeof meta>

/** Opened on mount so the canvas shows the menu; the trigger reopens it. */
export const Default: Story = {
  render: () => (
    <DropdownMenu defaultOpen>
      <DropdownMenuTrigger asChild>
        <Button variant="pill" size="sm">
          Session
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start">
        <DropdownMenuLabel>coder · 3 turns</DropdownMenuLabel>
        <DropdownMenuItem>Rename</DropdownMenuItem>
        <DropdownMenuItem>Duplicate</DropdownMenuItem>
        <DropdownMenuItem disabled>Archive</DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem className="text-alert data-[highlighted]:text-alert">
          Delete
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  ),
}

/** Icons lead, shortcuts trail in faint code ink; `align="end"` hugs a right-edge trigger. */
export const IconsAndShortcuts: Story = {
  render: () => (
    <DropdownMenu defaultOpen>
      <DropdownMenuTrigger asChild>
        <Button variant="pill" size="sm">
          Edit
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-[14rem]">
        <DropdownMenuItem>
          <Pencil aria-hidden className="size-4 text-ink-faint" />
          Rename
          <span className="ml-auto font-code text-[11px] text-ink-faint">
            ⌘R
          </span>
        </DropdownMenuItem>
        <DropdownMenuItem>
          <Copy aria-hidden className="size-4 text-ink-faint" />
          Copy path
          <span className="ml-auto font-code text-[11px] text-ink-faint">
            ⌘C
          </span>
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem className="text-alert data-[highlighted]:text-alert">
          <Trash2 aria-hidden className="size-4" />
          Delete
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  ),
}
