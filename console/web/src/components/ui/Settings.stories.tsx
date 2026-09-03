import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { Button } from './Button'
import { SettingsList, SettingsRow, SettingsSection } from './Settings'
import { Switch } from './Switch'

const meta = {
  title: 'UI/Settings',
  component: SettingsSection,
  parameters: { layout: 'padded' },
} satisfies Meta<typeof SettingsSection>

export default meta
type Story = StoryObj<typeof meta>

function NotificationsPreview() {
  const [critical, setCritical] = useState(true)
  const [system, setSystem] = useState(false)

  return (
    <div className="max-w-2xl">
      <SettingsSection
        title="Notifications"
        description="Choose which events should reach you."
      >
        <SettingsList>
          <SettingsRow
            label="Critical requests"
            description="Notify when a worker needs a critical decision."
            layout="inline"
            control={
              <Switch
                aria-label="Critical requests"
                name="critical-requests"
                checked={critical}
                onChange={(event) => setCritical(event.currentTarget.checked)}
              />
            }
          />
          <SettingsRow
            label="System notifications"
            description="Show a notification when a worker completes a task."
            layout="inline"
            control={
              <Switch
                aria-label="System notifications"
                name="system-notifications"
                checked={system}
                onChange={(event) => setSystem(event.currentTarget.checked)}
              />
            }
          />
        </SettingsList>
      </SettingsSection>
    </div>
  )
}

export const Notifications: Story = {
  render: () => <NotificationsPreview />,
}

export const KeyValueAndActions: Story = {
  render: () => (
    <div className="max-w-2xl">
      <SettingsSection
        title="Account"
        description="Identity and active-session controls."
        action={
          <Button type="button" variant="ghost" size="sm">
            Manage
          </Button>
        }
      >
        <SettingsList>
          <SettingsRow
            label="Email"
            control={<span>operator@example.com</span>}
          />
          <SettingsRow
            label="Device ID"
            meta="Current browser"
            control={<span className="font-mono">device-123</span>}
            action={
              <Button type="button" variant="ghost" size="sm">
                Copy
              </Button>
            }
          />
          <SettingsRow
            label="Log out from all devices"
            description="Ends every active session, including this one."
            action={
              <Button type="button" variant="ghost" size="sm">
                Log out
              </Button>
            }
          />
        </SettingsList>
      </SettingsSection>
    </div>
  ),
}

export const Narrow: Story = {
  render: () => (
    <div className="w-[320px]">
      <SettingsSection title="Storage">
        <SettingsList>
          <SettingsRow
            label="Archive directory"
            description="Files older than 30 days are moved automatically."
            control={<span className="font-mono">/archive/long/path</span>}
            action={
              <Button type="button" variant="ghost" size="sm">
                Change
              </Button>
            }
          />
        </SettingsList>
      </SettingsSection>
    </div>
  ),
}
