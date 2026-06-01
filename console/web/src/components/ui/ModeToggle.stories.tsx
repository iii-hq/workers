import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { ModeToggle } from './ModeToggle'

type Pick = 'a' | 'b' | 'c'

// ModeToggle is generic (`ModeToggle<T>`); stories drive their own state via
// `render`, so the `component` field is omitted to keep Meta typing simple.
const meta = {
  title: 'UI/ModeToggle',
  parameters: { layout: 'padded' },
} satisfies Meta

export default meta
type Story = StoryObj

export const TwoOption: Story = {
  name: 'two options',
  render: () => {
    const [value, setValue] = useState<'on' | 'off'>('on')
    return (
      <ModeToggle<'on' | 'off'>
        value={value}
        onChange={setValue}
        options={[
          { value: 'on', label: 'on' },
          { value: 'off', label: 'off' },
        ]}
      />
    )
  },
}

export const ThreeOption: Story = {
  name: 'three options',
  render: () => {
    const [value, setValue] = useState<Pick>('a')
    return (
      <ModeToggle<Pick>
        value={value}
        onChange={setValue}
        options={[
          { value: 'a', label: 'one' },
          { value: 'b', label: 'two' },
          { value: 'c', label: 'three' },
        ]}
      />
    )
  },
}
