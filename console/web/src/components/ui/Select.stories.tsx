import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { Select } from './Select'

type Basic = 'one' | 'two' | 'three'
type Region = 'us-east-1' | 'us-west-2' | 'eu-central-1' | 'ap-south-1'

const BASIC_OPTIONS = [
  { value: 'one' as const, label: 'option one' },
  { value: 'two' as const, label: 'option two' },
  { value: 'three' as const, label: 'option three' },
]

const REGION_GROUPS = [
  {
    label: 'americas',
    options: [
      { value: 'us-east-1' as const, label: 'us-east-1' },
      { value: 'us-west-2' as const, label: 'us-west-2' },
    ],
  },
  {
    label: 'europe',
    options: [{ value: 'eu-central-1' as const, label: 'eu-central-1' }],
  },
  {
    label: 'asia',
    options: [{ value: 'ap-south-1' as const, label: 'ap-south-1' }],
  },
]

const LONG_OPTIONS = [
  {
    value: 'long',
    label:
      'a very very long option label that should ellipsis instead of growing the trigger and breaking the layout',
  },
  {
    value: 's3',
    label:
      's3://my-very-long-bucket-name/path/to/some/deeply/nested/object/key.json',
  },
  { value: 'short', label: 'short' },
]

function Hint({ children }: { children: React.ReactNode }) {
  return (
    <p className="mt-2 font-mono text-[12px] leading-relaxed text-ink-faint lowercase">
      {children}
    </p>
  )
}

// Select is generic (`Select<T>`), so we skip the `component` field — wiring a
// generic function component into Storybook's Meta type fights the inference.
// Every story drives its own state via `render`, so args typing is unused.
const meta = {
  title: 'UI/Select',
  parameters: { layout: 'padded' },
} satisfies Meta

export default meta
type Story = StoryObj

export const Flat: Story = {
  name: 'flat options',
  render: () => {
    const [value, setValue] = useState<Basic>('one')
    return (
      <Select<Basic>
        value={value}
        onChange={setValue}
        options={BASIC_OPTIONS}
        aria-label="basic select"
        className="w-full max-w-[280px]"
      />
    )
  },
}

export const Grouped: Story = {
  name: 'grouped (provider-style)',
  render: () => {
    const [value, setValue] = useState<Region>('us-east-1')
    return (
      <Select<Region>
        value={value}
        onChange={setValue}
        groups={REGION_GROUPS}
        aria-label="region select"
        className="w-full max-w-[280px]"
      />
    )
  },
}

export const DisabledAndBusy: Story = {
  name: 'disabled + busy',
  render: () => {
    const [value, setValue] = useState<Basic>('one')
    return (
      <div className="flex flex-col gap-3 max-w-[280px]">
        <Select<Basic>
          value={value}
          onChange={setValue}
          options={BASIC_OPTIONS}
          disabled
          aria-label="disabled select"
          className="w-full"
        />
        <Select<Basic>
          value={value}
          onChange={setValue}
          options={BASIC_OPTIONS}
          aria-busy
          aria-label="loading catalog"
          className="w-full"
        />
      </div>
    )
  },
}

export const LongValueEllipsis: Story = {
  name: 'long value · ellipsis (fixed)',
  render: () => {
    const [value, setValue] = useState('long')
    return (
      <div className="max-w-[280px]">
        <Select<string>
          value={value}
          onChange={setValue}
          options={LONG_OPTIONS}
          aria-label="long value select"
          className="w-full"
        />
        <Hint>
          constrained to 280px. the label truncates with an ellipsis and the
          chevron stays put instead of being pushed off the edge.
        </Hint>
      </div>
    )
  },
}

export const UnsetPlaceholder: Story = {
  name: 'unset → placeholder (fixed)',
  render: () => {
    const [value, setValue] = useState<Basic | undefined>(undefined)
    return (
      <div className="max-w-[280px]">
        <Select<Basic>
          value={value}
          onChange={setValue}
          options={BASIC_OPTIONS}
          placeholder="select…"
          aria-label="unset select"
          className="w-full"
        />
        <Hint>
          starts as <code>undefined</code> → renders the placeholder, not the
          text "undefined". current: {value ?? '(unset)'}
        </Hint>
      </div>
    )
  },
}

export const AllowEmptyClearable: Story = {
  name: 'allowEmpty · clearable (fixed)',
  render: () => {
    const [value, setValue] = useState<Basic | undefined>('two')
    return (
      <div className="max-w-[280px]">
        <Select<Basic>
          value={value}
          onChange={setValue}
          onClear={() => setValue(undefined)}
          allowEmpty
          emptyLabel="unset"
          options={BASIC_OPTIONS}
          placeholder="select…"
          aria-label="clearable select"
          className="w-full"
        />
        <Hint>
          a leading "unset" option calls <code>onClear</code> to return to{' '}
          <code>undefined</code>. current: {value ?? '(unset)'}
        </Hint>
      </div>
    )
  },
}

export const UnmatchedValue: Story = {
  name: 'unmatched value → placeholder (fixed)',
  render: () => {
    const [, setValue] = useState<Basic>('one')
    return (
      <div className="max-w-[280px]">
        <Select<Basic>
          value={'does-not-exist' as Basic}
          onChange={setValue}
          options={BASIC_OPTIONS}
          placeholder="select…"
          aria-label="unmatched select"
          className="w-full"
        />
        <Hint>
          value is a stale id absent from the options; the trigger falls back to
          the placeholder rather than printing the raw value.
        </Hint>
      </div>
    )
  },
}
