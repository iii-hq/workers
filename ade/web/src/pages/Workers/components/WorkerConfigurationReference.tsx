import { errorMessage } from '@iii-dev/console-ui/format'
import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { Selector } from '@/components/ui/Selector'
import { SettingsList, SettingsRow } from '@/components/ui/Settings'
import { StatusPanel } from '@/components/ui/StatusPanel'
import {
  resolveSchema,
  schemaTypes,
} from '@/pages/Configuration/lib/schema/ref-resolver'
import type { ConfigurationSchemaView } from '@/pages/Configuration/tabs/WorkersTab/api'
import {
  loadWorkerConfigurationReference,
  type ReferenceCatalogReason,
  type WorkerConfigurationReferenceResult,
} from './worker-configuration-reference'

type ReferenceState =
  | WorkerConfigurationReferenceResult
  | { kind: 'loading' }
  | { kind: 'error'; detail: string }

const catalogHelp: Record<ReferenceCatalogReason, string> = {
  missing: 'This worker did not advertise a configuration reference.',
  ambiguous: 'This worker advertised more than one configuration identity.',
  'ambiguous-worker':
    'More than one live worker has this name; its exact configuration instance could not be verified.',
  unavailable:
    'No live worker with this name was found; its configuration identity could not be checked.',
  unverified:
    'The worker’s exact configuration instance could not be verified.',
}

/** Schema-only help beside the Compose override; no configuration values or edits. */
export function WorkerConfigurationReference({ name }: { name: string }) {
  const [requested, setRequested] = useState(false)
  const [attempt, setAttempt] = useState(0)
  const [state, setState] = useState<ReferenceState>({ kind: 'loading' })
  const [picked, setPicked] = useState<string | undefined>()
  // biome-ignore lint/correctness/useExhaustiveDependencies: Retry explicitly refetches the read-only reference.
  useEffect(() => {
    if (!requested) return
    let cancelled = false
    setState({ kind: 'loading' })
    setPicked(undefined)
    loadWorkerConfigurationReference(name)
      .then((next) => !cancelled && setState(next))
      .catch(
        (cause) =>
          !cancelled &&
          setState({ kind: 'error', detail: errorMessage(cause) }),
      )
    return () => {
      cancelled = true
    }
  }, [name, requested, attempt])
  const entry =
    state.kind === 'schema'
      ? state.entry
      : state.kind === 'catalog'
        ? state.entries.find((item) => item.id === picked)
        : undefined

  return (
    <details
      className="wk-config-reference"
      onToggle={(event) => {
        if (event.currentTarget.open) setRequested(true)
      }}
    >
      <summary>Configuration reference</summary>
      {requested ? (
        <div className="wk-config-reference-body">
          <p className="wk-note">
            Registered configuration keys, declared constraints and schema
            defaults. This reference does not show the current merged
            configuration or apply changes.
          </p>
          {state.kind === 'loading' ? (
            <p className="wk-note" role="status">
              Loading configuration reference…
            </p>
          ) : state.kind === 'error' ? (
            <StatusPanel
              className="wk-save-notice"
              role="alert"
              variant="alert"
              headline="Configuration reference unavailable"
              detail={state.detail}
              action={
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => setAttempt((next) => next + 1)}
                >
                  Retry
                </Button>
              }
            />
          ) : state.kind === 'catalog' ? (
            <>
              <p className="wk-note">
                {catalogHelp[state.reason]} Choose a registered entry to inspect
                its schema.
              </p>
              {state.entries.length ? (
                <Selector
                  aria-label="Configuration reference entry"
                  contentClassName="wk-reference-picker"
                  placeholder="Choose a registered entry…"
                  searchPlaceholder="Search configuration references"
                  value={picked}
                  onChange={setPicked}
                  options={state.entries.map((item) => ({
                    value: item.id,
                    label: item.name,
                    description: item.id,
                  }))}
                />
              ) : (
                <p className="wk-note">
                  No configuration references are registered.
                </p>
              )}
            </>
          ) : null}
          {entry ? <SchemaReference entry={entry} /> : null}
          <div className="wk-config-reference-example">
            <p className="wk-note">
              <strong>Override a nested setting</strong>
            </p>
            <p className="wk-note">
              Keep the parent key and indent its child keys with two spaces. Use
              the registered schema to choose the worker’s keys and value types.
            </p>
            <pre>
              <code>{'parent_key:\n  child_key: "example value"'}</code>
            </pre>
            <p className="wk-note">
              Illustrative YAML syntax only. Replace these keys and this value
              before adding an override; this example is not applied.
            </p>
          </div>
        </div>
      ) : null}
    </details>
  )
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

function declaredConstraints(
  field: Record<string, unknown>,
): [string, string][] {
  const constraints: [string, string][] = []
  for (const [key, label] of [
    ['minimum', 'Minimum'],
    ['maximum', 'Maximum'],
    ['exclusiveMinimum', 'Minimum (exclusive)'],
    ['exclusiveMaximum', 'Maximum (exclusive)'],
  ]) {
    const value = field[key]
    if (
      Object.hasOwn(field, key) &&
      typeof value === 'number' &&
      Number.isFinite(value)
    ) {
      constraints.push([label, String(value)])
    }
  }
  for (const [key, label] of [
    ['minLength', 'Min length'],
    ['maxLength', 'Max length'],
  ]) {
    const value = field[key]
    if (
      Object.hasOwn(field, key) &&
      typeof value === 'number' &&
      Number.isInteger(value) &&
      value >= 0
    ) {
      constraints.push([label, String(value)])
    }
  }
  if (Object.hasOwn(field, 'enum') && Array.isArray(field.enum)) {
    constraints.push(['Allowed values', JSON.stringify(field.enum)])
  }
  return constraints
}

function SchemaReference({ entry }: { entry: ConfigurationSchemaView }) {
  const root = entry.schema
  const schema = root ? resolveSchema(root, { root }) : null
  const fields =
    schema && isRecord(schema.properties)
      ? Object.entries(schema.properties)
      : []
  return (
    <div className="wk-config-reference-schema">
      <p className="wk-note">
        <strong>{entry.name}</strong>
        {' · '}
        <code className="wk-mono">{entry.id}</code>
      </p>
      {entry.description ? (
        <p className="wk-note">{entry.description}</p>
      ) : null}
      {root === null ? (
        <p className="wk-note">No configuration schema registered.</p>
      ) : (
        <>
          {fields.length ? (
            <SettingsList>
              {fields.map(([key, raw]) => {
                const field = isRecord(raw) ? resolveSchema(raw, { root }) : {}
                const type =
                  schemaTypes(field).join(' | ') || 'See complete schema'
                return (
                  <SettingsRow
                    key={key}
                    label={<code className="wk-mono">{key}</code>}
                    description={
                      typeof field.description === 'string'
                        ? field.description
                        : undefined
                    }
                    meta={
                      <div className="wk-config-reference-meta">
                        <span>
                          Type: <code className="wk-mono">{type}</code>
                        </span>
                        <span>
                          {Object.hasOwn(field, 'default') ? (
                            <>
                              Registered default:{' '}
                              <code className="wk-mono">
                                {JSON.stringify(field.default)}
                              </code>
                            </>
                          ) : (
                            'No declared default'
                          )}
                        </span>
                        {declaredConstraints(field).map(([label, value]) => (
                          <span key={label}>
                            {label}: <code className="wk-mono">{value}</code>
                          </span>
                        ))}
                        {schema &&
                        Object.hasOwn(schema, 'required') &&
                        Array.isArray(schema.required) &&
                        schema.required.includes(key) ? (
                          <span>Required in registered schema</span>
                        ) : null}
                      </div>
                    }
                  />
                )
              })}
            </SettingsList>
          ) : (
            <p className="wk-note">
              No top-level field list is registered. Inspect the complete schema
              below.
            </p>
          )}
          <details className="wk-config-reference-complete">
            <summary>Complete registered schema</summary>
            {/* biome-ignore lint/a11y/useSemanticElements: The named scroll region preserves preformatted schema text. */}
            <pre
              role="region"
              // biome-ignore lint/a11y/noNoninteractiveTabindex: Keyboard users must be able to scroll the complete schema.
              tabIndex={0}
              aria-label={`Complete registered schema for ${entry.name}`}
            >
              {JSON.stringify(root, null, 2)}
            </pre>
          </details>
        </>
      )}
    </div>
  )
}
