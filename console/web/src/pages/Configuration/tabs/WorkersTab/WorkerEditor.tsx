import { ArrowLeft } from 'lucide-react'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { Skeleton } from '@/components/ui/Skeleton'
import { configurationFormFamily } from '@/lib/configuration-family'
import {
  isExtConfigFormPending,
  useExtConfigForm,
  useUiAssetsStatus,
} from '@/lib/ui-slots'
import { cn } from '@/lib/utils'
import { isObjectSchema } from '../../lib/schema/guard'
import { type Path, pathToDomId } from '../../lib/schema/path'
import { validateConfig } from '../../lib/schema/validate'
import type { ConfigurationSchemaView, JsonValue } from './api'
import { isDirty } from './dirty'
import { EditorEmptyState } from './EmptyState'
import { parseSetError } from './errors'
import { useConfigurationValue, useSetConfiguration } from './hooks'
import { SaveBar, type SaveStatus } from './SaveBar'
import { wt } from './typography'

interface WorkerEditorProps {
  entry: ConfigurationSchemaView
  onDirtyChange: (dirty: boolean) => void
  /**
   * Drill-out affordance for the narrow (one-pane-at-a-time) flow: when
   * set, the header renders a ← back button that returns to the list.
   */
  onBack?: () => void
}

/**
 * Editor shell for a worker configuration entry. Loads the raw value
 * (templates preserved), hands the schema + value to the worker-owned form,
 * and owns the save lifecycle (mutation + status + dirty tracking + error
 * mapping). Every entry requires a registered form; the schema validates the
 * draft but never generates a visual fallback.
 *
 * Draft state: the editor holds the working copy locally so field edits
 * are responsive (no round-trip per keystroke). The draft is re-seeded
 * whenever a fresh value lands from the cache (mutation success, external
 * file edit, list refresh).
 *
 * Error mapping: server-side validation errors from `configuration::set`
 * come back as strings; `parseSetError` extracts a JSON Pointer when
 * possible. We hand the pointer→message map to the custom form so fields can
 * render errors inline, and always show the message in the save bar.
 *
 * Dirty flag: emitted up via `onDirtyChange` so the Configuration shell
 * can intercept tab switches and worker selection with the same
 * `useUnsavedGuard` instance that owns the `beforeunload` listener.
 */
export function WorkerEditor({
  entry,
  onDirtyChange,
  onBack,
}: WorkerEditorProps) {
  const valueQuery = useConfigurationValue(entry.id)
  const setMutation = useSetConfiguration(entry.id)
  // Injectable-UI configForms slot: a worker-registered form replaces the
  // FORM REGION only — the save lifecycle below stays host-owned either way.
  const formFamily = configurationFormFamily(entry)
  // An explicitly registered runtime-id form wins. Most workers register a
  // stable family form and advertise that family through metadata.ui_form so
  // instances renamed with III_CONFIG_NAME still receive the same UI.
  const formOverride = useExtConfigForm(entry.id, formFamily)
  const uiAssetsStatus = useUiAssetsStatus()
  const isFormOverrideLoading = isExtConfigFormPending(
    uiAssetsStatus,
    formOverride,
  )

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
  const fullForm = formOverride?.layout === 'full'

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
      className="flex-1 flex flex-col min-h-0 min-w-0 bg-panel"
      aria-label={`configuration ${entry.id}`}
    >
      <EditorHeader entry={entry} dirty={dirty} onBack={onBack} />
      <div
        className={cn(
          'flex flex-1 min-h-0 min-w-0 flex-col',
          fullForm ? 'overflow-hidden' : 'overflow-y-auto',
        )}
      >
        {valueQuery.isLoading ? <EditorLoading /> : null}
        {valueQuery.isError ? (
          <EditorError
            message={
              (valueQuery.error as Error)?.message ?? 'failed to load value'
            }
          />
        ) : null}
        {!valueQuery.isLoading && !valueQuery.isError && draft !== undefined ? (
          isFormOverrideLoading ? (
            <EditorLoading />
          ) : formOverride ? (
            <>
              <div
                className={cn(
                  'w-full min-w-0',
                  fullForm
                    ? 'flex flex-1 min-h-0 flex-col'
                    : 'mx-auto max-w-3xl px-6 py-8',
                )}
              >
                <formOverride.component
                  id={entry.id}
                  schema={isObjectSchema(entry.schema) ? entry.schema : null}
                  value={draft}
                  onChange={handleDraftChange}
                  errors={displayErrors}
                  focusField={fieldPathFromHash(entry.id)?.map(String)}
                />
                {rootError ? (
                  <p
                    className={cn(
                      wt.bodySm,
                      'text-alert',
                      fullForm
                        ? 'shrink-0 border-t border-edge px-4 py-3'
                        : 'mt-4',
                    )}
                    role="alert"
                  >
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
              title="Worker settings interface unavailable"
              description="This worker has not loaded a custom configuration interface. Start or enable it, then reopen settings."
            />
          )
        ) : null}
      </div>
    </section>
  )
}

/**
 * The settings pane's raised top bar — identity (name, dirty dot, id crumb,
 * description) plus the drill-out back button in the narrow flow.
 */
export function EditorHeader({
  entry,
  dirty = false,
  onBack,
}: {
  entry: ConfigurationSchemaView
  dirty?: boolean
  onBack?: () => void
}) {
  return (
    <header className="flex min-h-14 shrink-0 items-center gap-2 border-b border-edge bg-panel-raised py-2.5 pl-4 pr-12">
      {onBack ? (
        <button
          type="button"
          onClick={onBack}
          aria-label="Open settings navigation"
          className="relative -ml-1.5 flex size-10 shrink-0 items-center justify-center rounded-md text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus sm:size-8"
        >
          <span
            className="pointer-events-none absolute left-1/2 top-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
            aria-hidden="true"
          />
          <ArrowLeft className="size-4 shrink-0" aria-hidden />
        </button>
      ) : null}
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-2.5">
          <h2
            className={cn(
              wt.body,
              'truncate font-semibold text-ink tracking-[-0.01em]',
            )}
          >
            {entry.name || entry.id}
          </h2>
          {dirty ? (
            <span
              className="size-[7px] rounded-full bg-accent shrink-0"
              title="unsaved changes"
              aria-hidden
            />
          ) : null}
          <p className="truncate font-mono text-[0.75rem] text-ink-ghost">
            {entry.id}
          </p>
        </div>
        {entry.description ? (
          <p
            className={cn(wt.caption, 'text-ink-faint mt-0.5 truncate')}
            title={entry.description}
          >
            {entry.description}
          </p>
        ) : null}
      </div>
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

function fieldPathFromHash(workerId: string): Path | null {
  if (typeof window === 'undefined') return null
  const encodedWorkerId = encodeURIComponent(workerId)
  const prefixes = [
    `#/configuration/workers/${encodedWorkerId}/`,
    `#/workers/configuration/${encodedWorkerId}/`,
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
