import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { RawValueInput, type RawValueInputProps } from './RawValueInput'

type DemoProps = Pick<RawValueInputProps, 'kind' | 'disabled'>

const initial: Record<DemoProps['kind'], string> = {
  // biome-ignore lint/suspicious/noTemplateCurlyInString: environment references really are spelled `${VAR}`
  environment: '${OPENAI_API_KEY}',
  custom: '{{ secrets.openai.token }}',
}

function Demo({ kind, disabled }: DemoProps) {
  const [value, setValue] = useState(initial[kind])
  const [note, setNote] = useState('')
  return (
    <div className="flex max-w-lg flex-col gap-2">
      <RawValueInput
        kind={kind}
        label="API key"
        replacementLabel="literal"
        value={value}
        onChange={setValue}
        onUseLiteral={() =>
          setNote('Would swap the raw value for a typed literal input.')
        }
        disabled={disabled}
        aria-label="API key"
      />
      <span className="font-sans text-[12px] text-ink-faint">{note}</span>
    </div>
  )
}

const meta = {
  title: 'UI/RawValueInput',
  component: Demo,
  parameters: { layout: 'padded' },
  args: { kind: 'environment' },
} satisfies Meta<typeof Demo>

export default meta
type Story = StoryObj<typeof meta>

/** A `${VAR}` reference the console never expands; the button offers the literal path. */
export const Environment: Story = {}

export const Custom: Story = { args: { kind: 'custom' } }

export const Disabled: Story = { args: { disabled: true } }
