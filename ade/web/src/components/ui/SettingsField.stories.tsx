import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { Input } from './Input'
import { SettingsField, SettingsList, SettingsSection } from './Settings'
import { Switch } from './Switch'

/* Settings.stories covers Section/List/Row; this file is the labelled field
   that owns ids, error wiring and control widths. */
function Form({ error }: { error?: string }) {
  const [name, setName] = useState('browser')
  const [port, setPort] = useState('9222')
  const [headless, setHeadless] = useState(true)
  const [notes, setNotes] = useState('')
  return (
    <div className="max-w-2xl">
      <SettingsSection
        title="Browser worker"
        description="Each field renders its control at the width its value deserves."
      >
        <SettingsList>
          <SettingsField
            label="Worker name"
            description="Shown in the sidebar and in trace attribution."
            field="browser.name"
            error={error}
            renderControl={(props) => (
              <Input {...props} value={name} onChange={setName} />
            )}
          />
          <SettingsField
            label="Debug port"
            meta="compact"
            controlSize="compact"
            renderControl={(props) => (
              <Input
                {...props}
                value={port}
                onChange={setPort}
                inputMode="numeric"
              />
            )}
          />
          <SettingsField
            label="Headless"
            description="Run Chromium without a window."
            layout="inline"
            controlSize="fit"
            renderControl={(props) => (
              <Switch
                {...props}
                checked={headless}
                onChange={(event) => setHeadless(event.currentTarget.checked)}
              />
            )}
          />
          <SettingsField
            label="Launch notes"
            controlSize="full"
            layout="stacked"
            renderControl={(props) => (
              <Input
                {...props}
                value={notes}
                onChange={setNotes}
                placeholder="Free text, full width"
              />
            )}
          />
        </SettingsList>
      </SettingsSection>
    </div>
  )
}

const meta = {
  title: 'UI/SettingsField',
  component: Form,
  parameters: { layout: 'padded' },
} satisfies Meta<typeof Form>

export default meta
type Story = StoryObj<typeof meta>

/** `fit`, `compact`, `default` and `full` control widths side by side. */
export const ControlSizes: Story = {}

/** `error` renders under the meta slot and marks the control `aria-invalid`. */
export const WithError: Story = {
  args: { error: 'A worker with this name already exists.' },
}
