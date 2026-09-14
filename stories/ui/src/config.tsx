import {
  Button,
  type ConfigFormProps,
  Input,
  ListItem,
  SettingsDeck,
  SettingsField,
  SettingsList,
  SettingsRow,
  SettingsSection,
  Switch,
} from '@iii-dev/console-ui'
import { useEffect, useRef, useState } from 'react'
import { PlusIcon, TrashIcon } from './shared'

type WorkspaceDraft = {
  name: string
  path: string
  base?: string
  stories?: string[]
  ignore?: string[]
  [key: string]: unknown
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? { ...(value as Record<string, unknown>) } : {}
}

function readWorkspaces(record: Record<string, unknown>): WorkspaceDraft[] {
  const raw = Array.isArray(record.workspaces) ? record.workspaces : []
  return raw.map((entry) => {
    const item = asRecord(entry)
    return {
      ...item,
      name: typeof item.name === 'string' ? item.name : '',
      path: typeof item.path === 'string' ? item.path : '',
      base: typeof item.base === 'string' ? item.base : undefined,
      stories: Array.isArray(item.stories) ? item.stories.filter((s): s is string => typeof s === 'string') : undefined,
      ignore: Array.isArray(item.ignore) ? item.ignore.filter((s): s is string => typeof s === 'string') : undefined,
    }
  })
}

const csv = (values: string[] | undefined) => (values ?? []).join(', ')
const fromCsv = (value: string) =>
  value
    .split(',')
    .map((entry) => entry.trim())
    .filter((entry) => entry.length > 0)

function numberField(record: Record<string, unknown>, key: string, fallback: number): number {
  const value = record[key]
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback
}

export function StoriesConfigForm({ value, onChange, errors, focusField }: ConfigFormProps) {
  const record = asRecord(value)
  const viewport = asRecord(record.viewport)
  const workspaces = readWorkspaces(record)
  const dataPath = typeof record.data_path === 'string' ? record.data_path : ''
  const consoleUrl = typeof record.console_url === 'string' ? record.console_url : ''
  const watch = record.watch !== false
  const [active, setActive] = useState<number | null>(null)
  const consumed = useRef<string | null>(null)

  const focusKey = focusField && focusField.length > 0 ? focusField.map(String).join('.') : ''
  useEffect(() => {
    if (!focusKey || consumed.current === focusKey) return
    consumed.current = focusKey
    if (focusField && focusField[0] === 'workspaces') {
      const index = Number(focusField[1])
      if (Number.isInteger(index) && index >= 0 && index < workspaces.length) setActive(index)
    }
  }, [focusKey, focusField, workspaces.length])

  const update = (changes: Record<string, unknown>) => onChange({ ...record, ...changes } as never)
  const updateViewport = (changes: Record<string, unknown>) => update({ viewport: { ...viewport, ...changes } })
  const updateWorkspace = (index: number, changes: Partial<WorkspaceDraft>) =>
    update({ workspaces: workspaces.map((entry, position) => (position === index ? { ...entry, ...changes } : entry)) })
  const addWorkspace = () => {
    let suffix = workspaces.length + 1
    while (workspaces.some((entry) => entry.name === `workspace-${suffix}`)) suffix += 1
    const next = [...workspaces, { name: `workspace-${suffix}`, path: '.' }]
    update({ workspaces: next })
    setActive(next.length - 1)
  }
  const removeWorkspace = (index: number) => {
    update({ workspaces: workspaces.filter((_, position) => position !== index) })
    setActive(null)
  }
  const fieldFocus = (field: string): string | undefined => (focusKey === field ? field : undefined)
  const numberInput = (key: string, fallback: number, apply: (next: number) => void) => (
    <Input
      onChange={(next) => {
        const parsed = Number(next)
        if (Number.isFinite(parsed)) apply(parsed)
      }}
      type="number"
      value={String(numberField(key === 'keep_lines' ? record : viewport, key, fallback))}
    />
  )

  const current = active !== null ? workspaces[active] : undefined

  return (
    <>
      <SettingsSection
        description="Where snapshots live and how headless renders reach the console."
        title="Storage and rendering"
      >
        <SettingsList>
          <SettingsField
            data-field="data_path"
            description="Folder holding lines, files and renders. A relative path resolves from the project root."
            error={errors?.get('/data_path')}
            field={fieldFocus('data_path')}
            id="stories-data-path"
            label="Data folder"
            renderControl={(props) => (
              <Input
                {...props}
                onChange={(next) => update({ data_path: next })}
                placeholder="data/stories"
                value={dataPath}
              />
            )}
          />
          <SettingsField
            data-field="console_url"
            description="Origin the browser worker opens for screenshots and trees."
            error={errors?.get('/console_url')}
            field={fieldFocus('console_url')}
            id="stories-console-url"
            label="Console url"
            renderControl={(props) => (
              <Input
                {...props}
                onChange={(next) => update({ console_url: next })}
                placeholder="http://127.0.0.1:3113"
                value={consoleUrl}
              />
            )}
          />
          <SettingsField
            controlSize="fit"
            data-field="watch"
            description="Rebuild the working tree when a story input changes."
            error={errors?.get('/watch')}
            field={fieldFocus('watch')}
            id="stories-watch"
            label="Watch"
            layout="inline"
            renderControl={(props) => (
              <Switch {...props} checked={watch} onChange={(event) => update({ watch: event.currentTarget.checked })} />
            )}
          />
          <SettingsField
            data-field="keep_lines"
            description="Built ref lines kept per workspace before the oldest are pruned."
            error={errors?.get('/keep_lines')}
            field={fieldFocus('keep_lines')}
            id="stories-keep-lines"
            label="Keep lines"
            renderControl={(props) => (
              <Input
                {...props}
                onChange={(next) => {
                  const parsed = Number(next)
                  if (Number.isFinite(parsed)) update({ keep_lines: parsed })
                }}
                type="number"
                value={String(numberField(record, 'keep_lines', 12))}
              />
            )}
          />
          <SettingsField
            data-field="viewport.width"
            description="Default viewport for screenshots, in CSS pixels."
            error={errors?.get('/viewport/width')}
            field={fieldFocus('viewport.width')}
            id="stories-viewport-width"
            label="Viewport width"
            renderControl={() => numberInput('width', 1024, (next) => updateViewport({ width: next }))}
          />
          <SettingsField
            data-field="viewport.height"
            error={errors?.get('/viewport/height')}
            field={fieldFocus('viewport.height')}
            id="stories-viewport-height"
            label="Viewport height"
            renderControl={() => numberInput('height', 768, (next) => updateViewport({ height: next }))}
          />
          <SettingsField
            data-field="viewport.dpr"
            description="Device scale factor, 1 to 3."
            error={errors?.get('/viewport/dpr')}
            field={fieldFocus('viewport.dpr')}
            id="stories-viewport-dpr"
            label="Device scale"
            renderControl={() => numberInput('dpr', 1, (next) => updateViewport({ dpr: next }))}
          />
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        action={
          <Button onClick={addWorkspace} size="sm" variant="pill">
            <PlusIcon />
            Add workspace
          </Button>
        }
        description="Repositories whose story files are indexed. Projects inside each are discovered from the files."
        title="Workspaces"
      >
        <SettingsDeck
          autoFocusDetail={focusKey === ''}
          backLabel="Workspaces"
          detail={
            current && active !== null ? (
              <SettingsList>
                <SettingsField
                  data-field={`workspaces.${active}.name`}
                  description="Slug used in function calls and urls."
                  error={errors?.get(`/workspaces/${active}/name`)}
                  field={fieldFocus(`workspaces.${active}.name`)}
                  id="stories-workspace-name"
                  label="Name"
                  renderControl={(props) => (
                    <Input
                      {...props}
                      onChange={(next) =>
                        updateWorkspace(active, { name: next.toLowerCase().replace(/[^a-z0-9_-]/g, '-') })
                      }
                      value={current.name}
                    />
                  )}
                />
                <SettingsField
                  data-field={`workspaces.${active}.path`}
                  description="Repository root, relative to the project directory or absolute."
                  error={errors?.get(`/workspaces/${active}/path`)}
                  field={fieldFocus(`workspaces.${active}.path`)}
                  id="stories-workspace-path"
                  label="Path"
                  renderControl={(props) => (
                    <Input
                      {...props}
                      onChange={(next) => updateWorkspace(active, { path: next })}
                      placeholder="."
                      value={current.path}
                    />
                  )}
                />
                <SettingsField
                  data-field={`workspaces.${active}.base`}
                  description="Git ref the explorer compares the working tree against. Default HEAD."
                  error={errors?.get(`/workspaces/${active}/base`)}
                  field={fieldFocus(`workspaces.${active}.base`)}
                  id="stories-workspace-base"
                  label="Base ref"
                  renderControl={(props) => (
                    <Input
                      {...props}
                      onChange={(next) => updateWorkspace(active, { base: next.trim() === '' ? undefined : next })}
                      placeholder="HEAD"
                      value={current.base ?? ''}
                    />
                  )}
                />
                <SettingsField
                  data-field={`workspaces.${active}.stories`}
                  description="Comma-separated globs, relative to each project. Default **/*.stories.{tsx,ts,jsx,js}."
                  error={errors?.get(`/workspaces/${active}/stories`)}
                  field={fieldFocus(`workspaces.${active}.stories`)}
                  id="stories-workspace-stories"
                  label="Story globs"
                  renderControl={(props) => (
                    <Input
                      {...props}
                      onChange={(next) =>
                        updateWorkspace(active, { stories: next.trim() === '' ? undefined : fromCsv(next) })
                      }
                      placeholder="src/**/*.stories.tsx"
                      value={csv(current.stories)}
                    />
                  )}
                />
                <SettingsField
                  data-field={`workspaces.${active}.ignore`}
                  description="Comma-separated folder names or globs skipped while discovering."
                  error={errors?.get(`/workspaces/${active}/ignore`)}
                  field={fieldFocus(`workspaces.${active}.ignore`)}
                  id="stories-workspace-ignore"
                  label="Ignore"
                  renderControl={(props) => (
                    <Input
                      {...props}
                      onChange={(next) =>
                        updateWorkspace(active, { ignore: next.trim() === '' ? undefined : fromCsv(next) })
                      }
                      placeholder="fixtures, legacy/**"
                      value={csv(current.ignore)}
                    />
                  )}
                />
                <SettingsRow
                  action={
                    <Button
                      disabled={workspaces.length <= 1}
                      onClick={() => removeWorkspace(active)}
                      size="sm"
                      variant="ghost"
                    >
                      <TrashIcon />
                      Remove workspace
                    </Button>
                  }
                  description="Its snapshots stay on disk until pruned."
                  label="Remove"
                  layout="stacked"
                />
              </SettingsList>
            ) : null
          }
          onBack={() => setActive(null)}
          open={active !== null}
          overview={
            <SettingsList>
              {workspaces.length === 0 ? (
                <SettingsRow description="Add one to start indexing." label="No workspaces" />
              ) : null}
              {workspaces.map((entry, index) => (
                <ListItem
                  description={entry.path}
                  key={`${entry.name}-${index}`}
                  label={entry.name}
                  onClick={() => setActive(index)}
                />
              ))}
            </SettingsList>
          }
          title={current?.name ?? 'Workspace'}
        />
      </SettingsSection>
    </>
  )
}
