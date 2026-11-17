/**
 * "New key" / "New scope": one form, since a scope only exists while it
 * holds a key (see ./entries). Opened from the scopes column (scope empty,
 * focused) or the keys column (scope filled in, key focused). The value is
 * JSON, written with `state::compare-and-set` so an existing key is never
 * overwritten.
 */

import {
  type Host,
  ShadcnButton,
  ShadcnDialog,
  ShadcnDialogContent,
  ShadcnDialogDescription,
  ShadcnDialogFooter,
  ShadcnDialogHeader,
  ShadcnDialogTitle,
  ShadcnInput,
  ShadcnTextarea,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import type { ReactNode } from 'react'
import { useEffect, useId, useState } from 'react'
import { createEntry, KEY_EXISTS, validateNewEntry } from './entries'

export interface NewEntryRequest {
  /** `scope`: a new scope (its first key); `key`: a key in `scope`. */
  kind: 'scope' | 'key'
  scope: string
}

function Field({
  id,
  label,
  error,
  children,
}: {
  id: string
  label: string
  error?: string | null
  children: ReactNode
}) {
  return (
    <div className="state-ui-field">
      <label className="state-ui-field-label" htmlFor={id}>
        {label}
      </label>
      {children}
      {error ? (
        <p className="state-ui-field-error" id={`${id}-error`}>
          {error}
        </p>
      ) : null}
    </div>
  )
}

export function NewEntryDialog({
  host,
  request,
  onOpenChange,
  onCreated,
}: {
  host: Host
  request: NewEntryRequest | null
  onOpenChange: (open: boolean) => void
  onCreated: (entry: { scope: string; key: string }) => void
}) {
  const open = request !== null
  const [scope, setScope] = useState('')
  const [key, setKey] = useState('')
  const [value, setValue] = useState('{}')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  // Field errors show once the person tries to create, not while they type.
  const [tried, setTried] = useState(false)
  const uid = useId()

  useEffect(() => {
    if (!request) return
    setScope(request.kind === 'key' ? request.scope : '')
    setKey('')
    setValue('{}')
    setBusy(false)
    setError(null)
    setTried(false)
  }, [request])

  const draft = { scope, key, value }
  const fieldErrors = tried ? validateNewEntry(draft) : {}
  const keyError = fieldErrors.key ?? (error === KEY_EXISTS ? error : null)

  const submit = async () => {
    if (busy) return
    setTried(true)
    const errors = validateNewEntry(draft)
    if (errors.scope || errors.key || errors.value) return
    setBusy(true)
    setError(null)
    try {
      onCreated(await createEntry(host.iii, draft))
    } catch (caught) {
      setError(errorMessage(caught))
    } finally {
      setBusy(false)
    }
  }

  const isScope = request?.kind === 'scope'
  const scopeId = `${uid}-scope`
  const keyId = `${uid}-key`
  const valueId = `${uid}-value`

  return (
    <ShadcnDialog open={open} onOpenChange={onOpenChange}>
      <ShadcnDialogContent className="state-ui-dialog">
        <ShadcnDialogHeader>
          <ShadcnDialogTitle>{isScope ? 'New scope' : 'New key'}</ShadcnDialogTitle>
          <ShadcnDialogDescription>
            {isScope
              ? 'A scope holds keys. Give it a first key and value to create it.'
              : 'Store a JSON value under a new key in this scope.'}
          </ShadcnDialogDescription>
        </ShadcnDialogHeader>
        {/* Enter in a one-line field creates (the textarea keeps its newlines);
            the visible actions sit in the footer band. */}
        <form
          className="state-ui-form"
          onSubmit={(event) => {
            event.preventDefault()
            void submit()
          }}
          onKeyDown={(event) => {
            if (event.key !== 'Enter' || event.nativeEvent.isComposing) return
            if (!(event.target instanceof HTMLInputElement)) return
            event.preventDefault()
            void submit()
          }}
        >
          <Field id={scopeId} label="Scope" error={fieldErrors.scope}>
            <ShadcnInput
              id={scopeId}
              className="state-ui-input-mono"
              value={scope}
              readOnly={!isScope}
              autoFocus={isScope}
              autoComplete="off"
              spellCheck={false}
              placeholder="my-scope"
              aria-invalid={fieldErrors.scope ? true : undefined}
              aria-describedby={fieldErrors.scope ? `${scopeId}-error` : undefined}
              onChange={(event) => {
                setScope(event.target.value)
                setError(null)
              }}
            />
          </Field>
          <Field id={keyId} label="Key" error={keyError}>
            <ShadcnInput
              id={keyId}
              className="state-ui-input-mono"
              value={key}
              autoFocus={!isScope}
              autoComplete="off"
              spellCheck={false}
              placeholder="my-key"
              aria-invalid={keyError ? true : undefined}
              aria-describedby={keyError ? `${keyId}-error` : undefined}
              onChange={(event) => {
                setKey(event.target.value)
                setError(null)
              }}
            />
          </Field>
          <Field id={valueId} label="Value" error={fieldErrors.value}>
            <ShadcnTextarea
              id={valueId}
              className="state-ui-input-mono"
              value={value}
              rows={4}
              spellCheck={false}
              aria-invalid={fieldErrors.value ? true : undefined}
              aria-describedby={fieldErrors.value ? `${valueId}-error` : undefined}
              onChange={(event) => {
                setValue(event.target.value)
                setError(null)
              }}
            />
          </Field>
          {error && error !== KEY_EXISTS ? (
            <p className="state-ui-field-error" role="alert">
              {error}
            </p>
          ) : null}
        </form>
        <ShadcnDialogFooter>
          <ShadcnButton type="button" variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </ShadcnButton>
          <ShadcnButton type="button" disabled={busy} onClick={() => void submit()}>
            {busy ? 'Creating…' : isScope ? 'Create scope' : 'Create key'}
          </ShadcnButton>
        </ShadcnDialogFooter>
      </ShadcnDialogContent>
    </ShadcnDialog>
  )
}
