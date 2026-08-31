import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { ModeToggle } from './ModeToggle'

type Pick = 'data' | 'sql' | 'diagram'

// ModeToggle is generic (`ModeToggle<T>`); stories drive their own state via
// `render`, so the `component` field is omitted to keep Meta typing simple.
const meta = {
  title: 'UI/ModeToggle',
  parameters: { layout: 'padded' },
} satisfies Meta

export default meta
type Story = StoryObj

export const ContentTabs: Story = {
  name: 'content tabs',
  render: () => {
    const [value, setValue] = useState<Pick>('data')
    return (
      <ModeToggle<Pick>
        value={value}
        onChange={setValue}
        options={[
          { value: 'data', label: 'Data' },
          { value: 'sql', label: 'SQL' },
          { value: 'diagram', label: 'Diagram' },
        ]}
      />
    )
  },
}

export const PersistentChoice: Story = {
  name: 'persistent choice',
  render: () => {
    const [value, setValue] = useState<'light' | 'dark'>('dark')
    return (
      <ModeToggle<'light' | 'dark'>
        value={value}
        onChange={setValue}
        variant="radio"
        aria-label="Theme"
        options={[
          { value: 'light', label: 'Light' },
          { value: 'dark', label: 'Dark' },
        ]}
      />
    )
  },
}
