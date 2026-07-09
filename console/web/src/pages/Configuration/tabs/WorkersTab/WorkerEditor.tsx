import { useCallback, useEffect, useMemo, useState } from 'react'
import { Skeleton } from '@/components/ui/Skeleton'
import { cn } from '@/lib/utils'
import type { ConfigurationSchemaView, JsonValue } from './api'
import { isDirty } from './dirty'
import { EditorEmptyState } from './EmptyState'
import { parseSetError } from './errors'
import { useConfigurationValue, useSetConfiguration } from './hooks'
import { SaveBar, type SaveStatus } from './SaveBar'
import { isObjectSchema } from './schema-form/guard'
import { type Path, pathToDomId } from './schema-form/path'
import { SchemaForm } from './schema-form/SchemaForm'
import { validateConfig } from './schema-form/validate'
import { wt } from './typography'

interface WorkerEditorProps {
  entry: ConfigurationSchemaView
  onDirtyChange: (dirty: boolean) => void
}

/**
 * Editor shell for a worker configuration entry. Loads the raw value
 * (templates preserved), hands the schema + value to the `SchemaForm`, and
 * owns the save lifecycle (mutation + status + dirty tracking + error
 * mapping).
 *
 * Draft state: the editor holds the working copy locally so field edits
 * are responsive (no round-trip per keystroke). The draft is re-seeded
 * whenever a fresh value lands from the cache (mutation success, external
 * file edit, list refresh).
 *
 * Error mapping: server-side validation errors from `configuration::set`
 * come back as strings; `parseSetError` extracts a JSON Pointer when
 * possible. We hand the pointer→message map to `SchemaForm` so the
 * leaf field renders the error inline, and always show the message in
 * the save bar so the operator never loses sight of the rejection.
 *
 * Dirty flag: emitted up via `onDirtyChange` so the Configuration shell
 * can intercept tab switches and worker selection with the same
 * `useUnsavedGuard` instance that owns the `beforeunload` listener.
 */
export function WorkerEditor({ entry, onDirtyChange }: WorkerEditorProps) {
  const valueQuery = useConfigurationValue(entry.id)
  const setMutation = useSetConfiguration(entry.id)

  const [draft, setDraft] = useState<JsonValue | undefined>(undefined)
  const [status, setStatus] = useState<SaveStatus>({ kind: 'idle' })
  const [errors, setErrors] = useState<Map<string, string>>(new Map())

  // Seed / re-seed the draft from the loaded value. We track equality
  // against the previously-seeded reference so a re-render with the same
  // cached value doesn't wipe out the operator's in-progress edits.
  useEffect(() => {
    if (valueQuery.data === undefined) return
    setDraft(valueQuery.data)
    setStatus((cur) => (cur.kind === 'saving' ? cur : { kind: 'idle' }))
  }, [valueQuery.data])

  const dirty = useMemo(
    () => isDirty(valueQuery.data ?? null, draft ?? null),
    [valueQuery.data, draft],
  )

  // Client-side schema validation that mirrors the engine: env templates are
  // resolved/coerced before type-checking (so `${PORT:3111}` validates as an
  // integer), while a defaultless `${VAR}` is left for the runtime to resolve.
  // Derived from the live draft, so it can never go stale — unlike the server
  // `errors` below, which are cleared on edit.
  const clientErrors = useMemo(
    () =>
      draft === undefined || !isObjectSchema(entry.schema)
        ? new Map<string, string>()
        : validateConfig(draft, entry.schema),
    [draft, entry.schema],
  )

  // Client errors layer on top of any server error (client is always fresh).
  const displayErrors = useMemo(() => {
    const merged = new Map(errors)
    for (const [pointer, message] of clientErrors) merged.set(pointer, message)
    return merged
  }, [errors, clientErrors])

  // A root-level ('') validation error can't attach to any field; surface it
  // near the save bar so a disabled Save button always has a visible reason.
  const rootError = displayErrors.get('')

  // Bubble dirty changes up so the page shell can gate navigation.
  useEffect(() => {
    onDirtyChange(dirty)
  }, [dirty, onDirtyChange])

  useEffect(() => {
    if (draft === undefined) return
    const focusLinkedField = () => {
      const path = fieldPathFromHash(entry.id)
      if (!path) return
      const target = document.getElementById(pathToDomId(path))
      if (!target) return
      target.scrollIntoView({ block: 'center' })
      target.focus({ preventScroll: true })
    }
    const id = window.requestAnimationFrame(focusLinkedField)
    window.addEventListener('hashchange', focusLinkedField)
    return () => {
      window.cancelAnimationFrame(id)
      window.removeEventListener('hashchange', focusLinkedField)
    }
  }, [draft, entry.id])

  // Unmount cleanup: tell the shell we're no longer dirty when the
  // editor goes away (e.g. switching workers). Without this, the dirty
  // flag could persist into the next mount's first frame and block the
  // very navigation that triggered the unmount.
  useEffect(() => {
    return () => onDirtyChange(false)
  }, [onDirtyChange])

  const handleDraftChange = useCallback((next: JsonValue) => {
    setDraft(next)
    // Clear previous errors on edit so a stale validation message
    // doesn't linger past the fix.
    setErrors((cur) => (cur.size === 0 ? cur : new Map()))
    setStatus((cur) =>
      cur.kind === 'error' || cur.kind === 'saved' ? { kind: 'idle' } : cur,
    )
  }, [])

  const handleReset = useCallback(() => {
    if (valueQuery.data === undefined) return
    setDraft(valueQuery.data)
    setErrors(new Map())
    setStatus({ kind: 'idle' })
  }, [valueQuery.data])

  const handleSave = useCallback(() => {
    if (draft === undefined) return
    // Defensive: the Save button is disabled while client errors exist, but
    // guard here too in case a draft change races the click.
    if (clientErrors.size > 0) {
      setStatus({
        kind: 'error',
        message:
          clientErrors.get('') ?? 'fix the validation errors before saving',
      })
      return
    }
    setStatus({ kind: 'saving' })
    setErrors(new Map())
    setMutation.mutate(
      { id: entry.id, value: draft },
      {
        onSuccess: () => {
          setStatus({ kind: 'saved', savedAtMs: Date.now() })
          // The cache invalidation triggered by `useSetConfiguration`
          // will re-seed `draft` via the loaded-value effect; nothing
          // else to do here.
        },
        onError: (err) => {
          const parsed = parseSetError(err)
          setStatus({ kind: 'error', message: parsed.message })
          if (parsed.pointer) {
            setErrors(new Map([[parsed.pointer, parsed.message]]))
          }
        },
      },
    )
  }, [draft, entry.id, setMutation, clientErrors])

  return (
    <section
      className="flex-1 flex flex-col min-h-0 min-w-0"
      aria-label={`configuration ${entry.id}`}
    >
      <EditorHeader entry={entry} />
      <div className="flex-1 min-h-0 overflow-y-auto flex flex-col">
        {valueQuery.isLoading ? <EditorLoading /> : null}
        {valueQuery.isError ? (
          <EditorError
            message={
              (valueQuery.error as Error)?.message ?? 'failed to load value'
            }
          />
        ) : null}
        {!valueQuery.isLoading && !valueQuery.isError && draft !== undefined ? (
          isObjectSchema(entry.schema) ? (
            <>
              <div className="mx-auto max-w-3xl w-full px-6 py-8">
                <SchemaForm
                  schema={entry.schema}
                  value={draft}
                  onChange={handleDraftChange}
                  errors={displayErrors}
                />
                {rootError ? (
                  <p className={cn(wt.bodySm, 'text-alert mt-4')} role="alert">
                    {rootError}
                  </p>
                ) : null}
              </div>
              <SaveBar
                dirty={dirty}
                status={status}
                onSave={handleSave}
                onReset={handleReset}
                saveDisabled={clientErrors.size > 0}
              />
            </>
          ) : (
            <EditorEmptyState
              title="no editable configuration"
              description="this worker registered a configuration value but no schema, so there's nothing to edit here."
            />
          )
        ) : null}
      </div>
    </section>
  )
}

function EditorHeader({ entry }: { entry: ConfigurationSchemaView }) {
  return (
    <header className="px-6 py-4 border-b border-rule shrink-0">
      <div className="flex items-baseline gap-3">
        <h2
          className={cn(wt.heading, 'text-ink capitalize tracking-[-0.01em]')}
        >
          {entry.name || entry.id}
        </h2>
        <span className={cn(wt.caption, 'text-ink-ghost')}>{entry.id}</span>
      </div>
      {entry.description ? (
        <p className={cn(wt.bodySm, 'text-ink-faint mt-1 max-w-2xl')}>
          {entry.description}
        </p>
      ) : null}
    </header>
  )
}

function EditorLoading() {
  return (
    <div className="mx-auto max-w-3xl w-full px-6 py-8 space-y-4">
      <Skeleton className="h-5 w-40" />
      <Skeleton className="h-9 w-full" />
      <Skeleton className="h-9 w-full" />
      <Skeleton className="h-5 w-24 mt-6" />
      <Skeleton className="h-9 w-full" />
    </div>
  )
}

function EditorError({ message }: { message: string }) {
  return (
    <div className="mx-auto max-w-3xl w-full px-6 py-8">
      <p className={cn(wt.bodySm, 'text-alert')}>{message}</p>
    </div>
  )
}

/**
 * Re-export of the empty state used when no worker is selected. The
 * Workers tab decides whether to render this or a `WorkerEditor`; keeping
 * them in the same module collocates the two right-pane states so the
 * shell stays a thin dispatcher.
 */
export function WorkerEditorEmptySelection({
  hasEntries,
}: {
  hasEntries: boolean
}) {
  if (!hasEntries) {
    return (
      <EditorEmptyState
        title="no worker configurations registered"
        description="configurations are registered by workers at startup. start the workers you want to configure and they'll appear here."
      />
    )
  }
  return (
    <EditorEmptyState
      title="select a configuration"
      description="pick a worker on the left to view and edit its current value."
    />
  )
}

function fieldPathFromHash(workerId: string): Path | null {
  if (typeof window === 'undefined') return null
  const encodedWorkerId = encodeURIComponent(workerId)
  const prefixes = [
    `#/workers/configuration/${encodedWorkerId}/`,
    `#/configuration/workers/${encodedWorkerId}/`,
  ]
  const prefix = prefixes.find((p) => window.location.hash.startsWith(p))
  if (!prefix) return null
  const rest = window.location.hash.slice(prefix.length)
  const path = rest
    .split('/')
    .filter(Boolean)
    .map((segment) => decodeURIComponent(segment))
  return path.length > 0 ? path : null
}
