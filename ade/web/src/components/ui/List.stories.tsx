import type { Meta, StoryObj } from '@storybook/react-vite'
import { Bot, FileText, Globe, type LucideIcon, Terminal } from 'lucide-react'
import { useState } from 'react'
import { List, ListGroup, ListGroupLabel, ListItem } from './List'

interface Entry {
  id: string
  label: string
  description: string
  icon: LucideIcon
  meta?: string
  disabled?: boolean
}

const groups: { label: string; items: Entry[] }[] = [
  {
    label: 'Workers',
    items: [
      {
        id: 'browser',
        label: 'browser',
        description: 'Shared Chromium tabs',
        icon: Globe,
        meta: '2 tabs',
      },
      {
        id: 'ide',
        label: 'ide',
        description: 'Terminals, files and diffs',
        icon: Terminal,
      },
    ],
  },
  {
    label: 'Agents',
    items: [
      {
        id: 'coder',
        label: 'coder',
        description: 'Autonomous coding sessions',
        icon: Bot,
        meta: '3 live',
      },
      {
        id: 'docs',
        label: 'docs',
        description: 'Disabled until the worker starts',
        icon: FileText,
        disabled: true,
      },
    ],
  },
]

/** Arrow keys walk items across groups; Home/End jump to the ends. */
function Demo({ grouped = true }: { grouped?: boolean }) {
  const [selected, setSelected] = useState('browser')
  const renderItems = (items: Entry[]) =>
    items.map(({ icon: Icon, ...item }) => (
      <ListItem
        key={item.id}
        selected={selected === item.id}
        disabled={item.disabled}
        onClick={() => setSelected(item.id)}
        leading={<Icon aria-hidden className="size-4" />}
        label={item.label}
        description={item.description}
        trailing={item.meta}
      />
    ))
  return (
    <List aria-label="Catalog" className="w-72">
      {grouped
        ? groups.map((group) => (
            <ListGroup key={group.label}>
              <ListGroupLabel>{group.label}</ListGroupLabel>
              {renderItems(group.items)}
            </ListGroup>
          ))
        : renderItems(groups.flatMap((group) => group.items))}
    </List>
  )
}

const meta = {
  title: 'UI/List',
  component: Demo,
  parameters: { layout: 'padded' },
} satisfies Meta<typeof Demo>

export default meta
type Story = StoryObj<typeof meta>

export const Grouped: Story = {}
export const Flat: Story = { args: { grouped: false } }
