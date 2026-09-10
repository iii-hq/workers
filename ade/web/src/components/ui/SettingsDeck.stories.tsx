import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { Input } from './Input'
import { List, ListItem } from './List'
import { SettingsDeck } from './SettingsDeck'
import { Panel } from './Surface'

const meta = {
  title: 'UI/SettingsDeck',
  component: SettingsDeck,
  parameters: { layout: 'padded' },
} satisfies Meta<typeof SettingsDeck>

export default meta
type Story = StoryObj<typeof meta>

const connections = [
  { id: 'primary', label: 'Primary', description: 'PostgreSQL' },
  { id: 'analytics', label: 'Analytics', description: 'SQLite' },
]

function ConnectionDeckPreview() {
  const [activeId, setActiveId] = useState<string | null>(null)
  const [url, setUrl] = useState('postgres://localhost/app')
  const active = connections.find((connection) => connection.id === activeId)

  return (
    <div className="max-w-2xl">
      <SettingsDeck
        open={active !== undefined}
        title={active?.label ?? 'Connection'}
        description="Connection settings"
        backLabel="Connections"
        overview={
          <Panel>
            <List role="group" aria-label="Database connections">
              {connections.map((connection) => (
                <ListItem
                  key={connection.id}
                  label={connection.label}
                  description={connection.description}
                  trailing={<span aria-hidden>›</span>}
                  onClick={() => setActiveId(connection.id)}
                />
              ))}
            </List>
          </Panel>
        }
        detail={
          active ? (
            <label
              htmlFor="settings-deck-story-url"
              className="flex flex-col gap-2 font-sans text-sm text-ink"
            >
              Connection URL
              <Input
                id="settings-deck-story-url"
                value={url}
                onChange={setUrl}
              />
            </label>
          ) : null
        }
        onBack={() => setActiveId(null)}
      />
    </div>
  )
}

export const ResourceDrillIn: Story = {
  render: () => <ConnectionDeckPreview />,
  args: {
    open: false,
    overview: null,
    detail: null,
    title: 'Connection',
    onBack: () => {},
  },
}
