import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { SearchField } from './SearchField'

const meta = {
  title: 'UI/SearchField',
  component: SearchField,
  parameters: { layout: 'padded' },
} satisfies Meta<typeof SearchField>

export default meta
type Story = StoryObj<typeof meta>

function Controlled(props: { label?: string; initial?: string }) {
  const [value, setValue] = useState(props.initial ?? '')
  return (
    <div className="w-[20rem]">
      <SearchField
        value={value}
        onChange={setValue}
        placeholder="search functions…"
        label={props.label}
        aria-label={props.label ? undefined : 'Search functions'}
      />
    </div>
  )
}

/** Magnifier, placeholder, no clear button while empty. */
export const Empty: Story = {
  args: { value: '', onChange: () => {}, 'aria-label': 'Search' },
  render: () => <Controlled />,
}

/** Clear button appears once there is text; Escape also clears. */
export const WithText: Story = {
  args: { value: 'fs::', onChange: () => {}, 'aria-label': 'Search' },
  render: () => <Controlled initial="fs::" />,
}

export const VisibleLabel: Story = {
  args: { value: '', onChange: () => {}, label: 'Filter' },
  render: () => <Controlled label="Filter" />,
}
