import type { Meta, StoryObj } from '@storybook/react-vite'
import { Boxes, MessageSquareText, Radio, Terminal } from 'lucide-react'
import type { EngineSnapshot, PaletteEntry } from '@/lib/palette/sources'
import { CommandPalette } from './CommandPalette'

/* A gallery has no engine behind it, so the inventory is a fixture. Stable
   module-level identity on purpose: the palette re-reads whenever the reader
   changes. */
const INVENTORY: EngineSnapshot = {
  workers: [
    {
      name: 'shell',
      status: 'connected',
      version: '0.4.1',
      functionCount: 14,
    },
    {
      name: 'harness',
      status: 'connected',
      version: '1.8.2',
      functionCount: 31,
    },
    { name: 'iii-cron', status: 'available', version: '', functionCount: 4 },
    {
      name: 'database',
      status: 'disconnected',
      version: '0.4.0',
      functionCount: 9,
    },
  ],
  functions: [
    {
      id: 'shell::sessions::list',
      worker: 'shell',
      description: 'Every shell session the worker is holding open',
    },
    {
      id: 'shell::exec',
      worker: 'shell',
      description: 'Run one command and stream its output',
    },
    {
      id: 'engine::workers::list',
      worker: 'engine',
      description: 'Workers currently attached to the engine',
    },
  ],
  error: null,
}

const readInventory = () => Promise.resolve(INVENTORY)
const readEmptyInventory = () =>
  Promise.resolve({ workers: [], functions: [], error: null })
const readFailedInventory = () =>
  Promise.resolve({
    workers: [],
    functions: [],
    error: 'engine::workers::list timed out after 30000ms',
  })

const LOCAL_ENTRIES: PaletteEntry[] = [
  {
    id: 'action:new-chat',
    kind: 'action',
    title: 'New chat',
    detail: 'Start a conversation',
    run: () => {},
  },
  {
    id: 'action:settings',
    kind: 'action',
    title: 'Open settings',
    detail: 'Console and worker configuration',
    run: () => {},
  },
  {
    id: 'page:traces',
    kind: 'page',
    title: 'Traces',
    detail: 'Every run the engine has recorded',
    icon: Radio,
    run: () => {},
  },
  {
    id: 'page:workers',
    kind: 'page',
    title: 'Workers',
    detail: 'What is attached, and what it registers',
    icon: Boxes,
    run: () => {},
  },
  {
    id: 'page:shell',
    kind: 'page',
    title: 'Shell',
    detail: 'A terminal the agent shares',
    icon: Terminal,
    run: () => {},
  },
  {
    id: 'chat:1',
    kind: 'chat',
    title: 'why does the queue page show zero subscribers',
    detail: 'claude-opus-5',
    icon: MessageSquareText,
    run: () => {},
  },
  {
    id: 'chat:2',
    kind: 'chat',
    title: 'port the openwiki worker',
    detail: 'deepseek-v4-flash',
    icon: MessageSquareText,
    run: () => {},
  },
]

const meta = {
  title: 'Console/CommandPalette',
  component: CommandPalette,
  parameters: { layout: 'fullscreen' },
  args: {
    open: true,
    onClose: () => {},
    localEntries: LOCAL_ENTRIES,
    onOpenWorker: () => {},
    onOpenFunction: () => {},
    readInventory,
  },
} satisfies Meta<typeof CommandPalette>

export default meta
type Story = StoryObj<typeof meta>

/** Everything the console can reach, grouped. Narrow the browser under 640px
 *  to see the phone layout: full-bleed sheet, scrollable filters, a close
 *  affordance instead of `esc`. */
export const Default: Story = {}

/** Local rows only — the palette opens whether or not the engine answers. */
export const WithoutEngine: Story = {
  args: { readInventory: readEmptyInventory },
}

/** A read that failed says so and still shows what it does have. */
export const EngineUnavailable: Story = {
  args: { readInventory: readFailedInventory },
}

/** Nothing local either: the empty state has to carry the whole surface. */
export const NoResults: Story = {
  args: { localEntries: [], readInventory: readEmptyInventory },
}
