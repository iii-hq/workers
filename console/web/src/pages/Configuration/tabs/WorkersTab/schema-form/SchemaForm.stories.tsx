import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import type { JsonValue } from '@/pages/Configuration/tabs/WorkersTab/api'
import { WorkersTabDecorator } from '@/stories/decorators'
import {
  SCHEMA_EXAMPLES,
  type SchemaExample,
} from '@/stories/fixtures/worker-fixtures'
import { SchemaForm } from './SchemaForm'

/**
 * Read-only JSON mirror of the live form value, so each story shows that
 * every control round-trips back to the underlying `JsonValue` (`undefined`
 * keys drop, env templates stay verbatim, etc.).
 */
function ValuePreview({ value }: { value: JsonValue }) {
  return (
    <pre className="mt-3 border-t border-rule-2 pt-3 font-mono text-[12px] leading-relaxed text-ink-faint overflow-x-auto whitespace-pre-wrap">
      {JSON.stringify(value, null, 2)}
    </pre>
  )
}

function SchemaExampleCard({ example }: { example: SchemaExample }) {
  const [value, setValue] = useState<JsonValue>(example.initial)
  const [errors, setErrors] = useState<ReadonlyMap<string, string> | undefined>(
    example.errors,
  )

  function handleChange(next: JsonValue) {
    setValue(next)
    // Mirror the real editor: a server error clears as soon as the operator
    // edits past it.
    if (errors && errors.size > 0) setErrors(undefined)
  }

  return (
    <div className="max-w-[680px]">
      <SchemaForm
        schema={example.schema}
        value={value}
        onChange={handleChange}
        errors={errors}
      />
      <ValuePreview value={value} />
    </div>
  )
}

// Stories render via `SchemaExampleCard` (SchemaForm needs schema + value +
// onChange wired to local state), so the meta stays loose instead of
// declaring `component: SchemaForm`.
const meta = {
  title: 'Workers/SchemaForm',
  parameters: { layout: 'padded' },
  decorators: [WorkersTabDecorator],
} satisfies Meta

export default meta
type Story = StoryObj

function example(id: string): SchemaExample {
  const found = SCHEMA_EXAMPLES.find((e) => e.id === id)
  if (!found) throw new Error(`unknown schema example: ${id}`)
  return found
}

function storyFor(id: string): Story {
  const ex = example(id)
  return { name: ex.label, render: () => <SchemaExampleCard example={ex} /> }
}

export const StringText = storyFor('string')
export const StringEnvTemplate = storyFor('string-env')
export const StringPassword = storyFor('string-password')
export const StringDate = storyFor('string-date')
export const Integer = storyFor('integer')
export const NumberFractional = storyFor('number')
export const BooleanToggle = storyFor('boolean')
export const Enum = storyFor('enum')
export const EnumOptional = storyFor('enum-optional')
export const EnumNumeric = storyFor('enum-numeric')
export const OneOfCollapsed = storyFor('oneof-collapsed')
export const OneOfHeterogeneous = storyFor('oneof-hetero')
export const Nullable = storyFor('nullable')
export const ArrayPrimitives = storyFor('array-primitives')
export const ArrayObjects = storyFor('array-objects')
export const ObjectNested = storyFor('object')
export const Dictionary = storyFor('dictionary')
export const Ref = storyFor('ref')
export const Unsupported = storyFor('unsupported')
export const ValidationErrors = storyFor('errors')
