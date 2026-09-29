import { Button, Selector } from '@iii-dev/console-ui'
import type { SettingsFieldControlProps } from '@iii-dev/console-ui'
import { X } from 'lucide-react'

export interface WorkerSuggestion {
  value: string
  /** Why it is offered: where it is mapped now, or that it failed lately. */
  description?: string
}

/**
 * A set of worker names, edited as tokens.
 *
 * A comma-separated field asked people to remember and spell every name,
 * and cut the list off at the width of the input. Here each name is a token
 * that removes itself, and new ones are picked from the workers Sentinel has
 * actually seen — or typed, for a worker that has not failed yet.
 */
export function WorkerTokens({
  control,
  label,
  value,
  onChange,
  suggestions,
  empty,
}: {
  control: SettingsFieldControlProps
  label: string
  value: string[]
  onChange: (next: string[]) => void
  suggestions: WorkerSuggestion[]
  empty: string
}) {
  const options = suggestions
    .filter((suggestion) => !value.includes(suggestion.value))
    .map((suggestion) => ({ value: suggestion.value, label: suggestion.value, description: suggestion.description }))
  const add = (name: string) => {
    const trimmed = name.trim()
    if (trimmed && !value.includes(trimmed)) onChange([...value, trimmed])
  }

  return (
    <div className="sentinel-ui-tokens">
      {value.length > 0 ? (
        <ul className="sentinel-ui-token-list" aria-label={label}>
          {value.map((name) => (
            <li key={name}>
              <Button
                size="sm"
                variant="pill"
                aria-label={`Remove ${name}`}
                onClick={() => onChange(value.filter((other) => other !== name))}
              >
                <span className="sentinel-ui-mono">{name}</span>
                <X size={16} aria-hidden="true" />
              </Button>
            </li>
          ))}
        </ul>
      ) : (
        <span className="sentinel-ui-quiet">{empty}</span>
      )}
      <Selector
        id={control.id}
        data-field={control['data-field']}
        aria-describedby={control['aria-describedby']}
        aria-invalid={control['aria-invalid']}
        aria-label={`Add to ${label}`}
        value={undefined}
        options={options}
        onChange={add}
        onCreate={add}
        createOptionLabel={(query) => `add ${query}`}
        placeholder="Add a worker"
        searchPlaceholder="worker name"
        emptyMessage="No other worker has failed this week. Type a name to add it."
      />
    </div>
  )
}
