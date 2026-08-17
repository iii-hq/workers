import type { Meta, StoryObj } from '@storybook/react-vite'
import { useRef, useState } from 'react'
import { useContainerNarrow } from '@/hooks/use-container-narrow'
import {
  mockValidate,
  WORKER_CONFIG_FIXTURES,
} from '@/stories/fixtures/worker-fixtures'
import type { JsonValue } from './api'
import { isDirty } from './dirty'
import { EditorEmptyState } from './EmptyState'
import { SaveBar, type SaveStatus } from './SaveBar'
import { isObjectSchema } from './schema-form/guard'
import { SchemaForm } from './schema-form/SchemaForm'
import { EditorHeader } from './WorkerEditor'
import { WorkersList } from './WorkersList'

const VIEWS = WORKER_CONFIG_FIXTURES.map((f) => f.view)
const INITIAL_VALUES: Record<string, JsonValue> = Object.fromEntries(
  WORKER_CONFIG_FIXTURES.map((f) => [f.view.id, f.value]),
)

/** Mock `configuration::set` latency before the server "responds". */
const SAVE_LATENCY_MS = 600

/**
 * Self-contained reproduction of the worker-configuration master-detail editor.
 * Composes the real presentational pieces — `WorkersList`, `SchemaForm`,
 * `SaveBar`, `EditorEmptyState` — over local state and the
 * `WORKER_CONFIG_FIXTURES`, so the full edit / dirty / reset / save / error
 * flow is exercisable in Storybook without the live `configuration` worker.
 *
 * Save is simulated: after a short delay we run `mockValidate`; a rejection
 * maps a JSON Pointer onto the offending leaf (try saving `telemetry` with
 * `sample_rate > 1`, or clearing the `database` url), and a success commits
 * the draft to the per-id baseline so the dirty bar clears.
 */
function WorkerConfigHarness() {
  const [rootRef, narrow] = useContainerNarrow(720)
  const [selectedId, setSelectedId] = useState<string | null>(
    VIEWS[0]?.id ?? null,
  )
  const [baselines, setBaselines] =
    useState<Record<string, JsonValue>>(INITIAL_VALUES)
  const [drafts, setDrafts] =
    useState<Record<string, JsonValue>>(INITIAL_VALUES)
  const [status, setStatus] = useState<SaveStatus>({ kind: 'idle' })
  const [errors, setErrors] = useState<Map<string, string>>(new Map())

  // Track the live selection so an in-flight mock save doesn't splash its
  // status/error onto a worker the operator switched to mid-flight.
  const selectedRef = useRef(selectedId)
  selectedRef.current = selectedId

  const selectedView = selectedId
    ? (VIEWS.find((v) => v.id === selectedId) ?? null)
    : null
  const draft: JsonValue = selectedId ? (drafts[selectedId] ?? null) : null
  const baseline: JsonValue = selectedId
    ? (baselines[selectedId] ?? null)
    : null
  const dirty = selectedView ? isDirty(baseline, draft) : false

  function handleSelect(id: string | null) {
    setSelectedId(id)
    setStatus({ kind: 'idle' })
    setErrors(new Map())
  }

  function handleChange(next: JsonValue) {
    if (!selectedId) return
    setDrafts((cur) => ({ ...cur, [selectedId]: next }))
    if (errors.size > 0) setErrors(new Map())
    setStatus((cur) =>
      cur.kind === 'error' || cur.kind === 'saved' ? { kind: 'idle' } : cur,
    )
  }

  function handleReset() {
    if (!selectedId) return
    setDrafts((cur) => ({ ...cur, [selectedId]: baselines[selectedId] }))
    setErrors(new Map())
    setStatus({ kind: 'idle' })
  }

  function handleSave() {
    if (!selectedId) return
    const id = selectedId
    const value = drafts[id]
    setStatus({ kind: 'saving' })
    setErrors(new Map())
    window.setTimeout(() => {
      const invalid = mockValidate(id, value)
      if (invalid) {
        if (selectedRef.current === id) {
          setStatus({ kind: 'error', message: invalid.message })
          setErrors(new Map([[invalid.pointer, invalid.message]]))
        }
        return
      }
      setBaselines((cur) => ({ ...cur, [id]: value }))
      if (selectedRef.current === id) {
        setStatus({ kind: 'saved', savedAtMs: Date.now() })
      }
    }, SAVE_LATENCY_MS)
  }

  // Same drill-in rules as the real WorkersTab shell: under 720px the
  // list and editor take turns, with the editor header's ← going back.
  const showList = !narrow || selectedId === null
  const showEditor = !narrow || selectedId !== null

  return (
    <div className="border border-rule bg-bg h-full configuration-surface font-sans flex flex-col min-h-0">
      <div className="bg-panel px-3 py-1.5 border-b border-rule-2 font-sans text-[11px] uppercase tracking-[0.06em] text-ink-faint shrink-0">
        master-detail · mock fixtures
      </div>
      <div
        ref={rootRef}
        className="workers-tab flex-1 flex min-h-0 min-w-0 gap-px bg-edge"
      >
        {showList ? (
          <WorkersList
            configurations={VIEWS}
            selectedId={selectedId}
            onSelect={handleSelect}
            isLoading={false}
            isError={false}
            narrow={narrow}
          />
        ) : null}
        {showEditor ? (
          selectedView ? (
            <section
              className="flex-1 flex flex-col min-h-0 min-w-0 bg-panel"
              aria-label={`configuration ${selectedView.id}`}
            >
              <EditorHeader
                entry={selectedView}
                dirty={dirty}
                onBack={narrow ? () => handleSelect(null) : undefined}
              />
              <div className="flex-1 min-h-0 overflow-y-auto flex flex-col">
                {isObjectSchema(selectedView.schema) ? (
                  <>
                    <div className="mx-auto max-w-3xl w-full px-6 py-8">
                      <SchemaForm
                        key={selectedView.id}
                        schema={selectedView.schema}
                        value={draft}
                        onChange={handleChange}
                        errors={errors}
                      />
                    </div>
                    <SaveBar
                      dirty={dirty}
                      status={status}
                      onSave={handleSave}
                      onReset={handleReset}
                    />
                  </>
                ) : (
                  <EditorEmptyState
                    title="no editable configuration"
                    description="this worker registered a configuration value but no schema, so there's nothing to edit here."
                  />
                )}
              </div>
            </section>
          ) : (
            <EditorEmptyState
              title="select a configuration"
              description="pick a worker on the left to view and edit its current value."
            />
          )
        ) : null}
      </div>
    </div>
  )
}

const meta = {
  title: 'Workers/WorkersTab',
  component: WorkerConfigHarness,
  parameters: { layout: 'fullscreen' },
} satisfies Meta<typeof WorkerConfigHarness>

export default meta
type Story = StoryObj<typeof meta>

export const MasterDetail: Story = {
  name: 'master-detail editor',
  render: () => <WorkerConfigHarness />,
}

/** Width-constrained host: exercises the drill-in list ⇄ editor flow. */
export const NarrowDrillIn: Story = {
  name: 'narrow drill-in (mobile)',
  render: () => (
    <div className="h-full w-[390px]">
      <WorkerConfigHarness />
    </div>
  ),
}
